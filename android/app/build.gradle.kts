import java.net.URI
import java.security.MessageDigest
import java.util.zip.ZipInputStream
import javax.inject.Inject
import org.gradle.process.ExecOperations

plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
    id("org.jetbrains.kotlin.plugin.compose")
    id("org.jetbrains.kotlin.plugin.serialization")
}

// Embedded Node.js that runs the Zalo server on the phone (https://github.com/nodejs-mobile/nodejs-mobile).
val nodeMobileVersion = "v18.20.4"
val nodeMobileSha256 = "bd7321eaa1a7602fbe0bb87302df2d79d87835cf4363fbdd17c350dbb485c2af"
val nodeMobileDir = layout.buildDirectory.dir("nodejs-mobile")

android {
    lint {
        // AGP 8.7.3's lint crashes inside the lifecycle-runtime-android 2.9+ NullSafeMutableLiveData detector
        // (pulled in by the Compose BOM bump). The app does not use LiveData, so nothing is lost.
        disable += "NullSafeMutableLiveData"
    }
    namespace = "com.zalom3ui.app"
    compileSdk = 36

    defaultConfig {
        applicationId = "com.zalom3ui.app"
        minSdk = 26 // Android 8.0
        targetSdk = 35
        versionCode = 1
        versionName = "1.0.0"

        externalNativeBuild {
            cmake {
                arguments += "-DNODE_MOBILE_DIR=${nodeMobileDir.get().asFile.absolutePath}"
                // libnode.so (nodejs-mobile) links against the shared C++ runtime. With the shared STL the build packages
                // libc++_shared.so next to it; without it Node can't load ("libc++_shared.so" not found) and the app stays
                // on "Starting Zalo…".
                arguments += "-DANDROID_STL=c++_shared"
            }
        }
    }

    externalNativeBuild {
        cmake {
            path = file("src/main/cpp/CMakeLists.txt")
        }
    }

    // arm64 only (every current phone): libnode.so is ~60 MB per CPU type. The split keeps the APK's name
    // (app-arm64-v8a-debug.apk) and limits the native build to that one type.
    splits {
        abi {
            isEnable = true
            reset()
            include("arm64-v8a")
            isUniversalApk = false
        }
    }

    packaging {
        jniLibs {
            // Store native libraries uncompressed: Android loads them straight from the APK instead of unpacking a
            // second copy at install. libnode.so alone is ~50 MB unpacked, so the app takes ~17 MB less on the phone,
            // at the cost of a bigger download (~53 MB instead of ~19 MB).
            useLegacyPackaging = false
        }
    }

    buildTypes {
        release {
            isMinifyEnabled = true
            isShrinkResources = true
            proguardFiles(getDefaultProguardFile("proguard-android-optimize.txt"), "proguard-rules.pro")
            // Sideload-only app: sign release builds with the debug key unless you configure your own.
            signingConfig = signingConfigs.getByName("debug")
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
    kotlinOptions {
        jvmTarget = "17"
    }
    buildFeatures {
        compose = true
    }
}

/** Downloads the nodejs-mobile release, checks its hash, and extracts libnode.so and the headers. */
abstract class FetchNodeMobileTask : DefaultTask() {
    @get:Input abstract val version: Property<String>
    @get:Input abstract val sha256: Property<String>
    @get:Internal abstract val cacheDir: DirectoryProperty
    @get:OutputDirectory abstract val outputDir: DirectoryProperty

    @TaskAction
    fun fetch() {
        val v = version.get()
        val zip = cacheDir.file("nodejs-mobile-$v-android.zip").get().asFile
        if (!zip.exists() || sha256Of(zip) != sha256.get()) {
            zip.parentFile.mkdirs()
            val part = File(zip.path + ".part")
            val url = "https://github.com/nodejs-mobile/nodejs-mobile/releases/download/$v/nodejs-mobile-$v-android.zip"
            logger.lifecycle("Downloading $url")
            URI(url).toURL().openStream().use { input -> part.outputStream().use { input.copyTo(it) } }
            val actual = sha256Of(part)
            if (actual != sha256.get()) {
                part.delete()
                throw GradleException("nodejs-mobile download has SHA-256 $actual, expected ${sha256.get()}")
            }
            if (!part.renameTo(zip)) {
                part.copyTo(zip, overwrite = true)
                part.delete()
            }
            if (!zip.exists()) throw GradleException("Failed to move ${part.path} to ${zip.path}")
        }

        val out = outputDir.get().asFile
        out.deleteRecursively()
        val outPrefix = out.canonicalPath + File.separator
        ZipInputStream(zip.inputStream().buffered()).use { zin ->
            while (true) {
                val entry = zin.nextEntry ?: break
                val keep = !entry.isDirectory &&
                    ((entry.name.startsWith("bin/") && entry.name.endsWith("/libnode.so")) || entry.name.startsWith("include/"))
                if (!keep) continue
                val target = File(out, entry.name)
                require(target.canonicalPath.startsWith(outPrefix)) { "Bad zip entry ${entry.name}" }
                target.parentFile.mkdirs()
                target.outputStream().use { zin.copyTo(it) }
            }
        }
    }

    private fun sha256Of(file: File): String {
        val digest = MessageDigest.getInstance("SHA-256")
        file.inputStream().buffered().use { input ->
            val buf = ByteArray(1 shl 16)
            while (true) {
                val n = input.read(buf)
                if (n < 0) break
                digest.update(buf, 0, n)
            }
        }
        return digest.digest().joinToString("") { "%02x".format(it) }
    }
}

/** Bundles the server (../src) into one file with esbuild and stages it as an app asset. */
abstract class BundleServerTask : DefaultTask() {
    @get:Internal abstract val repoRoot: DirectoryProperty
    @get:InputFiles @get:PathSensitive(PathSensitivity.RELATIVE) abstract val serverSources: ConfigurableFileCollection
    @get:OutputDirectory abstract val outputDir: DirectoryProperty
    @get:Inject abstract val execOps: ExecOperations

    @TaskAction
    fun bundle() {
        val root = repoRoot.get().asFile
        val npm = if (System.getProperty("os.name").startsWith("Windows")) "npm.cmd" else "npm"
        val target = File(outputDir.get().asFile, "server/server.mjs")
        target.parentFile.mkdirs()
        execOps.exec {
            workingDir = root
            commandLine(npm, "run", "--silent", "bundle", "--", "--outfile=${target.absolutePath}")
        }
    }
}

val fetchNodeMobile = tasks.register<FetchNodeMobileTask>("fetchNodeMobile") {
    version.set(nodeMobileVersion)
    sha256.set(nodeMobileSha256)
    cacheDir.set(rootProject.layout.projectDirectory.dir(".nodejs-mobile"))
    outputDir.set(nodeMobileDir)
}

// CMake reads the headers and libnode.so from the extracted release.
tasks.configureEach {
    if (name.startsWith("configureCMake") || name.startsWith("buildCMake")) dependsOn(fetchNodeMobile)
}

androidComponents {
    onVariants { variant ->
        val bundle = tasks.register<BundleServerTask>("bundleServer${variant.name.replaceFirstChar { it.uppercase() }}") {
            val repo = rootProject.layout.projectDirectory.dir("..")
            repoRoot.set(repo)
            serverSources.from(repo.dir("src"), repo.file("package.json"), repo.file("package-lock.json"))
        }
        variant.sources.assets?.addGeneratedSourceDirectory(bundle, BundleServerTask::outputDir)
    }
}

dependencies {
    val composeBom = platform("androidx.compose:compose-bom:2025.10.00")
    implementation(composeBom)
    implementation("androidx.compose.ui:ui")
    implementation("androidx.compose.ui:ui-tooling-preview")
    debugImplementation("androidx.compose.ui:ui-tooling")

    // Material 3 Expressive (MaterialExpressiveTheme, MotionScheme, ButtonGroup, FloatingToolbar, LoadingIndicator, WavyProgressIndicator, MaterialShapes).
    implementation("androidx.compose.material3:material3:1.5.0-alpha13")
    implementation("androidx.compose.material:material-icons-extended")
    implementation("androidx.navigation:navigation-compose:2.8.5")
    implementation("io.coil-kt:coil-compose:2.7.0")
    // Animated stickers (WebP) and GIFs.
    implementation("io.coil-kt:coil-gif:2.7.0")

    implementation("androidx.core:core-ktx:1.15.0")
    // Rotation of picked photos, so they are sent with the size they are shown at.
    implementation("androidx.exifinterface:exifinterface:1.3.7")
    implementation("androidx.activity:activity-compose:1.9.3")
    implementation("androidx.lifecycle:lifecycle-viewmodel-compose:2.8.7")
    implementation("androidx.lifecycle:lifecycle-runtime-compose:2.8.7")

    implementation("com.squareup.okhttp3:okhttp:4.12.0")
    implementation("org.jetbrains.kotlinx:kotlinx-serialization-json:1.7.3")
    implementation("org.jetbrains.kotlinx:kotlinx-coroutines-android:1.9.0")

    testImplementation("junit:junit:4.13.2")
}
