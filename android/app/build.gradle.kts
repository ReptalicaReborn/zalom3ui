import javax.inject.Inject
import org.gradle.process.ExecOperations

plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
    id("org.jetbrains.kotlin.plugin.compose")
    id("org.jetbrains.kotlin.plugin.serialization")
}

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
        resourceConfigurations += listOf("en", "vi")
    }

    externalNativeBuild {
        cmake {
            path = file("src/main/cpp/CMakeLists.txt")
        }
    }

    // One APK per ABI, no universal APK: app-arm64-v8a-<type>.apk for almost every phone, and
    // app-armeabi-v7a-<type>.apk for 32-bit-only phones and phones running a 32-bit Android. The ABIs listed here are
    // also the ones the CMake build (Opus) targets. Do not add ndk { abiFilters }: AGP rejects it next to abi splits.
    splits {
        abi {
            isEnable = true
            reset()
            include("arm64-v8a", "armeabi-v7a")
            isUniversalApk = false
        }
    }

    packaging {
        jniLibs {
            // Compress native libraries in the APK (extracted on install) to keep the download small.
            useLegacyPackaging = true
        }
        resources {
            excludes += setOf(
                "DebugProbesKt.bin",
                "kotlin-tooling-metadata.json",
                "META-INF/androidx/**/LICENSE.txt",
                "META-INF/LICENSE*",
                "META-INF/NOTICE*",
            )
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

/**
 * One ABI the Rust core is built for: [abi] is the folder name inside the APK, [triple] the Rust target, [clangPrefix]
 * the NDK clang wrapper that compiles and links it (it says armv7a where the Rust target says armv7), and [rustflags]
 * extra RUSTFLAGS for that target only.
 */
class RustAbi(val abi: String, val triple: String, val clangPrefix: String, val rustflags: String?)

/** Compiles the embedded Rust Zalo server (libzalocore.so) for arm64-v8a and armeabi-v7a using the Android NDK toolchain. */
abstract class BuildRustCoreTask : DefaultTask() {
    @get:Internal abstract val rustDir: DirectoryProperty
    @get:Internal abstract val sdkDir: DirectoryProperty
    @get:InputFiles @get:PathSensitive(PathSensitivity.RELATIVE) abstract val rustSources: ConfigurableFileCollection
    @get:OutputDirectory abstract val outputDir: DirectoryProperty
    @get:Inject abstract val execOps: ExecOperations

    private fun resolveNdkDir(): File {
        val candidates = buildList {
            for (env in listOf("ANDROID_NDK_HOME", "ANDROID_NDK_ROOT", "ANDROID_NDK_LATEST_HOME")) {
                System.getenv(env)?.takeIf { it.isNotBlank() }?.let { add(File(it)) }
            }
            val sdkRoots = listOfNotNull(
                sdkDir.orNull?.asFile,
                System.getenv("ANDROID_HOME")?.takeIf { it.isNotBlank() }?.let { File(it) },
                System.getenv("ANDROID_SDK_ROOT")?.takeIf { it.isNotBlank() }?.let { File(it) },
            )
            for (sdk in sdkRoots) {
                File(sdk, "ndk").listFiles()
                    ?.filter { it.isDirectory }
                    ?.sortedByDescending { it.name }
                    ?.forEach { add(it) }
                add(File(sdk, "ndk-bundle"))
            }
        }
        return candidates.firstOrNull { File(it, "toolchains/llvm/prebuilt").isDirectory }
            ?: throw GradleException(
                "Could not locate Android NDK (checked ANDROID_NDK_* env vars and <sdk>/ndk/*)",
            )
    }

    @TaskAction
    fun build() {
        val rust = rustDir.get().asFile
        val ndk = resolveNdkDir()
        val isWindows = System.getProperty("os.name").startsWith("Windows", ignoreCase = true)
        val arName = if (isWindows) "llvm-ar.exe" else "llvm-ar"
        val cargo = if (isWindows) "cargo.exe" else "cargo"
        val prebuiltRoot = File(ndk, "toolchains/llvm/prebuilt")

        // Keep this list in step with `splits { abi { include(...) } }` above: every ABI that gets an APK needs its core.
        val abis = listOf(
            RustAbi(
                abi = "arm64-v8a",
                triple = "aarch64-linux-android",
                clangPrefix = "aarch64-linux-android26",
                // Hardware AES / GHASH (ARMv8 Crypto Extensions) for message crypto and call SRTP. RustCrypto
                // only builds them in with these cfgs; at runtime they are used if the CPU has them and fall back
                // to software otherwise. Without the cfgs every AES block ran in software.
                rustflags = "--cfg aes_armv8 --cfg polyval_armv8",
            ),
            // 32-bit ARM (Cortex-A7 and the like): no hardware AES, so none of those cfgs (they are aarch64 only).
            RustAbi(
                abi = "armeabi-v7a",
                triple = "armv7-linux-androideabi",
                clangPrefix = "armv7a-linux-androideabi26",
                rustflags = null,
            ),
        )

        for (target in abis) {
            val clangName = if (isWindows) "${target.clangPrefix}-clang.cmd" else "${target.clangPrefix}-clang"
            val llvmBin = prebuiltRoot.listFiles()
                ?.filter { it.isDirectory }
                ?.map { File(it, "bin") }
                ?.firstOrNull { File(it, clangName).isFile && File(it, arName).isFile }
                ?: throw GradleException(
                    "Could not locate $clangName and $arName under ${prebuiltRoot.absolutePath}",
                )

            val clang = File(llvmBin, clangName).absolutePath
            val llvmAr = File(llvmBin, arName).absolutePath
            // aarch64-linux-android -> AARCH64_LINUX_ANDROID / aarch64_linux_android, the spelling cargo and the cc crate read.
            val tripleLower = target.triple.replace('-', '_')
            val tripleUpper = tripleLower.uppercase()
            val rustflags = target.rustflags
            val triple = target.triple

            execOps.exec {
                workingDir = rust
                environment("CARGO_TARGET_${tripleUpper}_LINKER", clang)
                environment("CC_$tripleLower", clang)
                environment("AR_$tripleLower", llvmAr)
                if (rustflags != null) {
                    environment("CARGO_TARGET_${tripleUpper}_RUSTFLAGS", rustflags)
                }
                commandLine(cargo, "build", "--release", "--target", triple, "-p", "zalo-android")
            }

            val built = File(rust, "target/${target.triple}/release/libzalocore.so")
            if (!built.isFile) {
                throw GradleException("Expected compiled Rust library not found at ${built.absolutePath}")
            }
            val dest = File(outputDir.get().asFile, "${target.abi}/libzalocore.so")
            dest.parentFile.mkdirs()
            built.copyTo(dest, overwrite = true)
        }
    }
}

androidComponents {
    onVariants { variant ->
        val buildRustCore = tasks.register<BuildRustCoreTask>(
            "buildRustCore${variant.name.replaceFirstChar { it.uppercase() }}",
        ) {
            val rustRoot = rootProject.layout.projectDirectory.dir("../rust")
            rustDir.set(rustRoot)
            sdkDir.set(androidComponents.sdkComponents.sdkDirectory)
            rustSources.from(
                project.fileTree(rustRoot) {
                    include("Cargo.toml", "Cargo.lock", "zalo-*/Cargo.toml", "zalo-*/src/**")
                },
            )
        }
        variant.sources.jniLibs?.addGeneratedSourceDirectory(buildRustCore, BuildRustCoreTask::outputDir)
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
    // ProcessLifecycleOwner: the app coming to and leaving the foreground, for "Stay connected: While open".
    implementation("androidx.lifecycle:lifecycle-process:2.8.7")

    // Installs the baseline profile (app rules in src/main/baseline-prof.txt + the libraries' own) on first launch,
    // so ART compiles the startup path ahead of time: sideloaded APKs get no Play Store cloud profile.
    implementation("androidx.profileinstaller:profileinstaller:1.4.1")

    implementation("com.squareup.okhttp3:okhttp:4.12.0")
    implementation("org.jetbrains.kotlinx:kotlinx-serialization-json:1.7.3")
    implementation("org.jetbrains.kotlinx:kotlinx-coroutines-android:1.9.0")

    testImplementation("junit:junit:4.13.2")
}
