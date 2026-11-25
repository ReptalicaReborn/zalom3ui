package com.zalom3ui.app.call

import android.Manifest
import android.annotation.SuppressLint
import android.content.Context
import android.content.pm.PackageManager
import android.graphics.SurfaceTexture
import android.hardware.camera2.CameraCaptureSession
import android.hardware.camera2.CameraCharacteristics
import android.hardware.camera2.CameraDevice
import android.hardware.camera2.CameraManager
import android.hardware.camera2.CaptureRequest
import android.hardware.display.DisplayManager
import android.media.MediaCodec
import android.media.MediaCodecInfo
import android.media.MediaFormat
import android.os.Bundle
import android.os.Handler
import android.os.HandlerThread
import android.util.Range
import android.util.Size
import android.view.Display
import android.view.Surface
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import kotlin.math.abs

/**
 * Our camera as H.264 for the call: Camera2 -> [CameraGl] (rotates the picture upright) -> a [MediaCodec] encoder, whose
 * output goes to [onFrame] as Annex B access units with SPS/PPS in front of every key frame. A small self-view can be
 * drawn on a surface from [setPreviewSurface].
 *
 * Choices, mirroring Zalos where it has one (640x480 maximum, about 15 fps, a key frame every couple of seconds):
 * - The frame is always portrait, 480x640, because phone calls are held upright and Zalo's format has no rotation flag.
 *   The camera captures 640x480 (sensor shape). If the phone is turned sideways the picture is rotated to stay upright
 *   and gets black bars.
 * - Constrained baseline, CBR, 500 kbit/s, 15 fps, key frame every 2 s and on [requestKeyFrame].
 * - The front camera is sent as the camera sees it (not mirrored); only the self-view is mirrored. Camera2 hands the
 *   texture over already rotated for the sensor and mirrored for a front camera, so only the screen's rotation and the
 *   mirror are undone here.
 *
 * Needs the CAMERA permission already granted. [onFrame] and [onError] run on worker threads.
 */
class CameraVideoSender(
    context: Context,
    private val onFrame: (key: Boolean, accessUnit: ByteArray) -> Unit,
    private val onError: (message: String) -> Unit = {},
) {
    private val app = context.applicationContext
    private val cameras = app.getSystemService(CameraManager::class.java)

    // Guarded by this, except where the GL thread owns it.
    private var encoder: MediaCodec? = null
    private var gl: CameraGl? = null
    private var glThread: HandlerThread? = null
    @Volatile private var glHandler: Handler? = null
    private var drainThread: Thread? = null
    @Volatile private var running = false
    private val failing = java.util.concurrent.atomic.AtomicBoolean(false)
    @Volatile private var previewSurface: Surface? = null

    // Owned by the GL thread.
    private var texture: SurfaceTexture? = null
    private var textureSurface: Surface? = null
    private var device: CameraDevice? = null
    private var session: CameraCaptureSession? = null
    private var front = true
    private var sensorOrientation = 90
    private var camSize = Size(CAPTURE_W, CAPTURE_H)
    private var generation = 0

    @Volatile private var config: ByteArray? = null

    val isRunning: Boolean get() = running

    /**
     * Opens the encoder and the camera. Throws if the permission is missing or the encoder can't be built; camera
     * trouble after this point comes through [onError], and the sender stops itself.
     */
    @Synchronized
    fun start(frontCamera: Boolean = true) {
        if (running) return
        check(app.checkSelfPermission(Manifest.permission.CAMERA) == PackageManager.PERMISSION_GRANTED) { "Camera permission is missing" }
        val enc = createEncoder()
        val input: Surface
        try {
            input = enc.createInputSurface()
            enc.start()
        } catch (e: Exception) {
            runCatching { enc.release() }
            throw e
        }
        encoder = enc
        config = null
        failing.set(false)
        front = frontCamera
        running = true
        drainThread = Thread({ drain(enc) }, "camera-encoder").apply { isDaemon = true; start() }
        val t = HandlerThread("camera-gl").apply { start() }
        glThread = t
        val h = Handler(t.looper)
        glHandler = h
        h.post { setUp(input, h) }
    }

    @Synchronized
    fun stop() {
        if (!running) return
        running = false
        val h = glHandler
        val t = glThread
        if (h != null && t != null) {
            val done = CountDownLatch(1)
            h.post {
                tearDown()
                done.countDown()
            }
            done.await(2, TimeUnit.SECONDS)
            t.quitSafely()
            runCatching { t.join(1000) }
        }
        glHandler = null
        glThread = null
        drainThread?.let { runCatching { it.join(1000) } }
        drainThread = null
        encoder?.let {
            runCatching { it.stop() }
            runCatching { it.release() }
        }
        encoder = null
    }

    /** Changes between the front and the back camera. */
    fun switchCamera() {
        glHandler?.post {
            front = !front
            openCamera()
        }
    }

    /** Encodes a key frame now (the other side lost one). */
    fun requestKeyFrame() {
        val e = encoder ?: return
        runCatching { e.setParameters(Bundle().apply { putInt(MediaCodec.PARAMETER_KEY_REQUEST_SYNC_FRAME, 0) }) }
    }

    /**
     * The surface of the small self-view, or null when it goes away; may be called before or after [start]. With null
     * this returns after the sender stopped drawing on the old surface.
     */
    fun setPreviewSurface(surface: Surface?) {
        previewSurface = surface
        val h = glHandler ?: return
        val done = CountDownLatch(1)
        h.post {
            try {
                gl?.setPreview(surface)
            } finally {
                done.countDown()
            }
        }
        if (surface == null) done.await(500, TimeUnit.MILLISECONDS)
    }

    private fun createEncoder(): MediaCodec {
        fun format(profile: Boolean): MediaFormat = MediaFormat.createVideoFormat(MIME, OUT_W, OUT_H).apply {
            setInteger(MediaFormat.KEY_COLOR_FORMAT, MediaCodecInfo.CodecCapabilities.COLOR_FormatSurface)
            setInteger(MediaFormat.KEY_BIT_RATE, BITRATE)
            setInteger(MediaFormat.KEY_FRAME_RATE, FPS)
            setInteger(MediaFormat.KEY_I_FRAME_INTERVAL, KEY_FRAME_SECONDS)
            setInteger(MediaFormat.KEY_BITRATE_MODE, MediaCodecInfo.EncoderCapabilities.BITRATE_MODE_CBR)
            if (profile) setInteger(MediaFormat.KEY_PROFILE, PROFILE_CONSTRAINED_BASELINE)
        }
        for (withProfile in listOf(true, false)) {
            val c = MediaCodec.createEncoderByType(MIME)
            try {
                c.configure(format(withProfile), null, null, MediaCodec.CONFIGURE_FLAG_ENCODE)
                return c
            } catch (e: Exception) {
                runCatching { c.release() }
                if (!withProfile) throw e
            }
        }
        error("unreachable")
    }

    /** Takes encoded frames out of the encoder until it is released. */
    private fun drain(enc: MediaCodec) {
        val info = MediaCodec.BufferInfo()
        try {
            while (running) {
                val i = enc.dequeueOutputBuffer(info, 100_000)
                if (i < 0) continue
                val buf = enc.getOutputBuffer(i)
                if (buf != null && info.size > 0) {
                    buf.position(info.offset)
                    buf.limit(info.offset + info.size)
                    val data = ByteArray(info.size)
                    buf.get(data)
                    if (info.flags and MediaCodec.BUFFER_FLAG_CODEC_CONFIG != 0) {
                        // SPS + PPS in Annex B (csd-0 and csd-1 together); sent in front of each key frame.
                        config = data
                    } else {
                        val key = info.flags and MediaCodec.BUFFER_FLAG_KEY_FRAME != 0
                        onFrame(key, AnnexB.withParameterSets(config, key, data))
                    }
                }
                enc.releaseOutputBuffer(i, false)
            }
        } catch (e: Exception) {
            // Stopping releases the codec under us; only a real failure is worth reporting.
            if (running) fail("The video encoder stopped: ${e.message}")
        }
    }

    // ---- GL thread ----

    private fun setUp(encoderInput: Surface, handler: Handler) {
        try {
            val g = CameraGl(encoderInput)
            g.init()
            gl = g
            g.setPreview(previewSurface)
            val st = SurfaceTexture(g.textureId)
            st.setOnFrameAvailableListener({ onFrameAvailable(it) }, handler)
            texture = st
            textureSurface = Surface(st)
            openCamera()
        } catch (e: Exception) {
            fail("Couldn't start the camera: ${e.message}")
        }
    }

    private fun onFrameAvailable(st: SurfaceTexture) {
        val g = gl ?: return
        if (!running || st !== texture) return
        try {
            st.updateTexImage()
            // The texture comes already turned by the sensor orientation: its upright size swaps for a 90°/270° sensor.
            val turnedSensor = sensorOrientation % 180 == 90
            val w = if (turnedSensor) camSize.height else camSize.width
            val h = if (turnedSensor) camSize.width else camSize.height
            // Front camera: the texture is mirrored; the video we send isn't (as other apps send it), the self-view is.
            g.draw(st, w, h, rotation(), frontCamera = front, outWidth = OUT_W, outHeight = OUT_H)
        } catch (e: Exception) {
            fail("Camera drawing failed: ${e.message}")
        }
    }

    /**
     * Clockwise degrees that make the (already sensor-corrected) texture upright on the screen as it's turned now, which
     * is upright in the real world since the call screen follows the device. Only the display rotation is left to undo:
     * Camera2 applies the sensor orientation itself for SurfaceTexture outputs (rotating by it again turned the picture
     * 90° on a real call, from a tablet whose front camera sits on the long edge).
     */
    private fun rotation(): Int {
        val display = app.getSystemService(DisplayManager::class.java).getDisplay(Display.DEFAULT_DISPLAY)
        val d = when (display?.rotation) {
            Surface.ROTATION_90 -> 90
            Surface.ROTATION_180 -> 180
            Surface.ROTATION_270 -> 270
            else -> 0
        }
        return (360 - d) % 360
    }

    @SuppressLint("MissingPermission") // checked in start()
    private fun openCamera() {
        val mine = ++generation
        closeCamera()
        try {
            val id = pickCamera() ?: run {
                fail("No camera found")
                return
            }
            val chars = cameras.getCameraCharacteristics(id)
            sensorOrientation = chars.get(CameraCharacteristics.SENSOR_ORIENTATION) ?: 90
            camSize = pickSize(chars)
            texture?.setDefaultBufferSize(camSize.width, camSize.height)
            val handler = glHandler ?: return
            cameras.openCamera(
                id,
                object : CameraDevice.StateCallback() {
                    override fun onOpened(camera: CameraDevice) {
                        if (mine != generation || !running) {
                            camera.close()
                            return
                        }
                        device = camera
                        startSession(camera, chars, mine)
                    }

                    override fun onDisconnected(camera: CameraDevice) {
                        camera.close()
                        if (mine == generation && running) fail("The camera was disconnected")
                    }

                    override fun onError(camera: CameraDevice, error: Int) {
                        camera.close()
                        if (mine == generation && running) fail("The camera failed (error $error)")
                    }
                },
                handler,
            )
        } catch (e: Exception) {
            fail("Couldn't open the camera: ${e.message}")
        }
    }

    @Suppress("DEPRECATION") // the SessionConfiguration form needs API 28; minSdk is 26
    private fun startSession(camera: CameraDevice, chars: CameraCharacteristics, mine: Int) {
        val target = textureSurface ?: return
        try {
            camera.createCaptureSession(
                listOf(target),
                object : CameraCaptureSession.StateCallback() {
                    override fun onConfigured(s: CameraCaptureSession) {
                        if (mine != generation || !running) {
                            s.close()
                            return
                        }
                        session = s
                        try {
                            val req = camera.createCaptureRequest(CameraDevice.TEMPLATE_RECORD).apply {
                                addTarget(target)
                                pickFpsRange(chars)?.let { set(CaptureRequest.CONTROL_AE_TARGET_FPS_RANGE, it) }
                                set(CaptureRequest.CONTROL_AF_MODE, CaptureRequest.CONTROL_AF_MODE_CONTINUOUS_VIDEO)
                            }
                            s.setRepeatingRequest(req.build(), null, glHandler)
                        } catch (e: Exception) {
                            fail("Couldn't start the camera: ${e.message}")
                        }
                    }

                    override fun onConfigureFailed(s: CameraCaptureSession) {
                        if (mine == generation && running) fail("The camera couldn't be configured")
                    }
                },
                glHandler,
            )
        } catch (e: Exception) {
            fail("Couldn't start the camera: ${e.message}")
        }
    }

    private fun closeCamera() {
        session?.let { runCatching { it.close() } }
        session = null
        device?.let { runCatching { it.close() } }
        device = null
    }

    private fun tearDown() {
        generation++
        closeCamera()
        textureSurface?.release()
        textureSurface = null
        texture?.let {
            it.setOnFrameAvailableListener(null)
            it.release()
        }
        texture = null
        gl?.release()
        gl = null
    }

    private fun pickCamera(): String? {
        val want = if (front) CameraCharacteristics.LENS_FACING_FRONT else CameraCharacteristics.LENS_FACING_BACK
        val ids = cameras.cameraIdList
        return ids.firstOrNull { cameras.getCameraCharacteristics(it).get(CameraCharacteristics.LENS_FACING) == want }
            ?: ids.firstOrNull()
    }

    /** The camera size closest to 640x480 among 4:3 sizes (any size if the camera has none). */
    private fun pickSize(chars: CameraCharacteristics): Size {
        val sizes = chars.get(CameraCharacteristics.SCALER_STREAM_CONFIGURATION_MAP)
            ?.getOutputSizes(SurfaceTexture::class.java)?.toList().orEmpty()
        val fourThree = sizes.filter { it.width * 3 == it.height * 4 }
        return (fourThree.ifEmpty { sizes }).minByOrNull { abs(it.width * it.height - CAPTURE_W * CAPTURE_H) }
            ?: Size(CAPTURE_W, CAPTURE_H)
    }

    /** About 20 fps at most, with the lowest floor available so a dim room slows the camera down instead of the picture. */
    private fun pickFpsRange(chars: CameraCharacteristics): Range<Int>? {
        val ranges = chars.get(CameraCharacteristics.CONTROL_AE_AVAILABLE_TARGET_FPS_RANGES) ?: return null
        return ranges.filter { it.upper in FPS..30 }.minWithOrNull(compareBy<Range<Int>>({ abs(it.upper - 20) }, { it.lower }))
    }

    private fun fail(message: String) {
        if (!running || !failing.compareAndSet(false, true)) return
        // Stop on a different thread: stop() waits for the GL thread, which may be the one reporting.
        Thread {
            stop()
            onError(message)
        }.start()
    }

    private companion object {
        const val MIME = "video/avc"
        const val OUT_W = 480
        const val OUT_H = 640
        const val CAPTURE_W = 640
        const val CAPTURE_H = 480
        const val FPS = 15
        const val BITRATE = 500_000
        const val KEY_FRAME_SECONDS = 2

        // MediaCodecInfo.CodecProfileLevel.AVCProfileConstrainedBaseline, spelled out because it is newer than minSdk.
        const val PROFILE_CONSTRAINED_BASELINE = 0x10000
    }
}
