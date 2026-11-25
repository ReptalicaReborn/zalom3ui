package com.zalom3ui.app.call

import android.graphics.SurfaceTexture
import android.opengl.EGL14
import android.opengl.EGLConfig
import android.opengl.EGLContext
import android.opengl.EGLDisplay
import android.opengl.EGLExt
import android.opengl.EGLSurface
import android.opengl.GLES11Ext
import android.opengl.GLES20
import android.opengl.Matrix
import android.view.Surface
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.nio.FloatBuffer

/**
 * Draws camera frames (an external texture) onto the encoder's input surface and, when there is one, the self-view
 * surface. The camera hands over frames in sensor orientation; this is where they are turned upright (and fitted into
 * the encoder's portrait frame, or cropped to fill the preview) because the video format has no rotation flag.
 *
 * Everything here must run on one thread, the one that calls [init].
 */
internal class CameraGl(private val encoderSurface: Surface) {
    private var display: EGLDisplay = EGL14.EGL_NO_DISPLAY
    private var context: EGLContext = EGL14.EGL_NO_CONTEXT
    private var config: EGLConfig? = null
    private var encoderEgl: EGLSurface = EGL14.EGL_NO_SURFACE
    private var previewEgl: EGLSurface = EGL14.EGL_NO_SURFACE
    private var program = 0
    private var aPos = 0
    private var aTex = 0
    private var uMvp = 0
    private var uTex = 0
    var textureId = 0
        private set

    private val quad: FloatBuffer = floatBuffer(floatArrayOf(-1f, -1f, 1f, -1f, -1f, 1f, 1f, 1f))
    private val texCoords: FloatBuffer = floatBuffer(floatArrayOf(0f, 0f, 1f, 0f, 0f, 1f, 1f, 1f))
    private val proj = FloatArray(16)
    private val model = FloatArray(16)
    private val mvp = FloatArray(16)
    private val flip = FloatArray(16)
    private val size = IntArray(1)

    fun init() {
        display = EGL14.eglGetDisplay(EGL14.EGL_DEFAULT_DISPLAY)
        check(display != EGL14.EGL_NO_DISPLAY) { "no EGL display" }
        val version = IntArray(2)
        check(EGL14.eglInitialize(display, version, 0, version, 1)) { "EGL init failed" }
        val attribs = intArrayOf(
            EGL14.EGL_RED_SIZE, 8,
            EGL14.EGL_GREEN_SIZE, 8,
            EGL14.EGL_BLUE_SIZE, 8,
            EGL14.EGL_RENDERABLE_TYPE, EGL14.EGL_OPENGL_ES2_BIT,
            EGLExt.EGL_RECORDABLE_ANDROID, 1,
            EGL14.EGL_NONE,
        )
        val configs = arrayOfNulls<EGLConfig>(1)
        val count = IntArray(1)
        check(EGL14.eglChooseConfig(display, attribs, 0, configs, 0, 1, count, 0) && count[0] > 0) { "no EGL config" }
        config = configs[0]
        context = EGL14.eglCreateContext(
            display, config, EGL14.EGL_NO_CONTEXT, intArrayOf(EGL14.EGL_CONTEXT_CLIENT_VERSION, 2, EGL14.EGL_NONE), 0,
        )
        check(context != EGL14.EGL_NO_CONTEXT) { "no EGL context" }
        encoderEgl = windowSurface(encoderSurface) ?: error("no EGL surface for the encoder")
        makeCurrent(encoderEgl)

        program = buildProgram()
        aPos = GLES20.glGetAttribLocation(program, "aPos")
        aTex = GLES20.glGetAttribLocation(program, "aTex")
        uMvp = GLES20.glGetUniformLocation(program, "uMvp")
        uTex = GLES20.glGetUniformLocation(program, "uTexMatrix")
        val tex = IntArray(1)
        GLES20.glGenTextures(1, tex, 0)
        textureId = tex[0]
        GLES20.glBindTexture(GLES11Ext.GL_TEXTURE_EXTERNAL_OES, textureId)
        GLES20.glTexParameteri(GLES11Ext.GL_TEXTURE_EXTERNAL_OES, GLES20.GL_TEXTURE_MIN_FILTER, GLES20.GL_LINEAR)
        GLES20.glTexParameteri(GLES11Ext.GL_TEXTURE_EXTERNAL_OES, GLES20.GL_TEXTURE_MAG_FILTER, GLES20.GL_LINEAR)
        GLES20.glTexParameteri(GLES11Ext.GL_TEXTURE_EXTERNAL_OES, GLES20.GL_TEXTURE_WRAP_S, GLES20.GL_CLAMP_TO_EDGE)
        GLES20.glTexParameteri(GLES11Ext.GL_TEXTURE_EXTERNAL_OES, GLES20.GL_TEXTURE_WRAP_T, GLES20.GL_CLAMP_TO_EDGE)
    }

    /** Points the self-view at [surface] (null removes it). A surface that can't be drawn on is ignored. */
    fun setPreview(surface: Surface?) {
        if (previewEgl != EGL14.EGL_NO_SURFACE) {
            // Leave the surface before destroying it, then go back to the encoder's.
            EGL14.eglMakeCurrent(display, EGL14.EGL_NO_SURFACE, EGL14.EGL_NO_SURFACE, EGL14.EGL_NO_CONTEXT)
            EGL14.eglDestroySurface(display, previewEgl)
            previewEgl = EGL14.EGL_NO_SURFACE
            makeCurrent(encoderEgl)
        }
        if (surface != null && surface.isValid) previewEgl = windowSurface(surface) ?: EGL14.EGL_NO_SURFACE
    }

    /**
     * Draws the latest [SurfaceTexture] frame. The texture's own transform already turns the camera picture upright for
     * the device's natural orientation (and mirrors a front camera): Camera2 sets that buffer transform from the sensor
     * orientation. [camWidth] x [camHeight] is the picture's size in that natural orientation, [rotation] the clockwise
     * degrees that make it upright on the screen as it's turned now. For a [frontCamera] the texture's mirror is undone
     * first, in the texture's own orientation (undoing it after a 90° turn would flip the picture upside down instead):
     * the video we send is unmirrored, and the self-view is then mirrored on screen like a selfie.
     */
    fun draw(texture: SurfaceTexture, camWidth: Int, camHeight: Int, rotation: Int, frontCamera: Boolean, outWidth: Int, outHeight: Int) {
        val st = FloatArray(16)
        texture.getTransformMatrix(st)
        drawTo(encoderEgl, st, camWidth, camHeight, rotation, outWidth, outHeight, frontCamera, mirror = false, cover = false, nanos = texture.timestamp)
        if (previewEgl != EGL14.EGL_NO_SURFACE) {
            EGL14.eglQuerySurface(display, previewEgl, EGL14.EGL_WIDTH, size, 0)
            val w = size[0]
            EGL14.eglQuerySurface(display, previewEgl, EGL14.EGL_HEIGHT, size, 0)
            val h = size[0]
            if (w > 0 && h > 0) drawTo(previewEgl, st, camWidth, camHeight, rotation, w, h, frontCamera, mirror = frontCamera, cover = true, nanos = 0)
            // The encoder's surface stays current between frames.
            makeCurrent(encoderEgl)
        }
    }

    private fun drawTo(
        target: EGLSurface,
        st: FloatArray,
        camWidth: Int,
        camHeight: Int,
        rotation: Int,
        w: Int,
        h: Int,
        unmirrorTexture: Boolean,
        mirror: Boolean,
        cover: Boolean,
        nanos: Long,
    ) {
        makeCurrent(target)
        GLES20.glViewport(0, 0, w, h)
        GLES20.glClearColor(0f, 0f, 0f, 1f)
        GLES20.glClear(GLES20.GL_COLOR_BUFFER_BIT)

        // Unit quad -> camera size in pixels -> rotated (clockwise = negative in GL) -> scaled to fit or fill -> clip space.
        val turned = rotation == 90 || rotation == 270
        val rw = (if (turned) camHeight else camWidth).toFloat()
        val rh = (if (turned) camWidth else camHeight).toFloat()
        val scale = if (cover) maxOf(w / rw, h / rh) else minOf(w / rw, h / rh)
        Matrix.orthoM(proj, 0, -w / 2f, w / 2f, -h / 2f, h / 2f, -1f, 1f)
        Matrix.setIdentityM(model, 0)
        Matrix.scaleM(model, 0, scale, scale, 1f)
        Matrix.rotateM(model, 0, -rotation.toFloat(), 0f, 0f, 1f)
        // Undo the front camera's mirror in the texture's own (natural) orientation, before the turn.
        if (unmirrorTexture) Matrix.scaleM(model, 0, -1f, 1f, 1f)
        Matrix.scaleM(model, 0, camWidth / 2f, camHeight / 2f, 1f)
        Matrix.multiplyMM(mvp, 0, proj, 0, model, 0)
        if (mirror) {
            Matrix.setIdentityM(flip, 0)
            Matrix.scaleM(flip, 0, -1f, 1f, 1f)
            val tmp = mvp.copyOf()
            Matrix.multiplyMM(mvp, 0, flip, 0, tmp, 0)
        }

        GLES20.glUseProgram(program)
        GLES20.glActiveTexture(GLES20.GL_TEXTURE0)
        GLES20.glBindTexture(GLES11Ext.GL_TEXTURE_EXTERNAL_OES, textureId)
        GLES20.glUniformMatrix4fv(uMvp, 1, false, mvp, 0)
        GLES20.glUniformMatrix4fv(uTex, 1, false, st, 0)
        GLES20.glEnableVertexAttribArray(aPos)
        GLES20.glVertexAttribPointer(aPos, 2, GLES20.GL_FLOAT, false, 8, quad)
        GLES20.glEnableVertexAttribArray(aTex)
        GLES20.glVertexAttribPointer(aTex, 2, GLES20.GL_FLOAT, false, 8, texCoords)
        GLES20.glDrawArrays(GLES20.GL_TRIANGLE_STRIP, 0, 4)
        GLES20.glDisableVertexAttribArray(aPos)
        GLES20.glDisableVertexAttribArray(aTex)

        if (nanos > 0) EGLExt.eglPresentationTimeANDROID(display, target, nanos)
        EGL14.eglSwapBuffers(display, target)
    }

    fun release() {
        if (display == EGL14.EGL_NO_DISPLAY) return
        EGL14.eglMakeCurrent(display, EGL14.EGL_NO_SURFACE, EGL14.EGL_NO_SURFACE, EGL14.EGL_NO_CONTEXT)
        if (previewEgl != EGL14.EGL_NO_SURFACE) EGL14.eglDestroySurface(display, previewEgl)
        if (encoderEgl != EGL14.EGL_NO_SURFACE) EGL14.eglDestroySurface(display, encoderEgl)
        if (context != EGL14.EGL_NO_CONTEXT) EGL14.eglDestroyContext(display, context)
        EGL14.eglReleaseThread()
        EGL14.eglTerminate(display)
        display = EGL14.EGL_NO_DISPLAY
        context = EGL14.EGL_NO_CONTEXT
        previewEgl = EGL14.EGL_NO_SURFACE
        encoderEgl = EGL14.EGL_NO_SURFACE
    }

    private fun windowSurface(s: Surface): EGLSurface? {
        val surface = EGL14.eglCreateWindowSurface(display, config, s, intArrayOf(EGL14.EGL_NONE), 0)
        if (surface == null || surface == EGL14.EGL_NO_SURFACE) return null
        return surface
    }

    private fun makeCurrent(surface: EGLSurface) {
        check(EGL14.eglMakeCurrent(display, surface, surface, context)) { "eglMakeCurrent failed" }
        // Never wait for the screen's refresh: frames go out as the camera makes them.
        EGL14.eglSwapInterval(display, 0)
    }

    private fun buildProgram(): Int {
        val vs = compile(
            GLES20.GL_VERTEX_SHADER,
            """
            uniform mat4 uMvp;
            uniform mat4 uTexMatrix;
            attribute vec4 aPos;
            attribute vec4 aTex;
            varying vec2 vTex;
            void main() {
                gl_Position = uMvp * aPos;
                vTex = (uTexMatrix * aTex).xy;
            }
            """.trimIndent(),
        )
        val fs = compile(
            GLES20.GL_FRAGMENT_SHADER,
            """
            #extension GL_OES_EGL_image_external : require
            precision mediump float;
            varying vec2 vTex;
            uniform samplerExternalOES sTex;
            void main() {
                gl_FragColor = texture2D(sTex, vTex);
            }
            """.trimIndent(),
        )
        val p = GLES20.glCreateProgram()
        GLES20.glAttachShader(p, vs)
        GLES20.glAttachShader(p, fs)
        GLES20.glLinkProgram(p)
        val ok = IntArray(1)
        GLES20.glGetProgramiv(p, GLES20.GL_LINK_STATUS, ok, 0)
        check(ok[0] == GLES20.GL_TRUE) { "GL link failed: " + GLES20.glGetProgramInfoLog(p) }
        return p
    }

    private fun compile(type: Int, source: String): Int {
        val s = GLES20.glCreateShader(type)
        GLES20.glShaderSource(s, source)
        GLES20.glCompileShader(s)
        val ok = IntArray(1)
        GLES20.glGetShaderiv(s, GLES20.GL_COMPILE_STATUS, ok, 0)
        check(ok[0] == GLES20.GL_TRUE) { "GL shader failed: " + GLES20.glGetShaderInfoLog(s) }
        return s
    }

    private fun floatBuffer(v: FloatArray): FloatBuffer =
        ByteBuffer.allocateDirect(v.size * 4).order(ByteOrder.nativeOrder()).asFloatBuffer().apply { put(v).position(0) }
}
