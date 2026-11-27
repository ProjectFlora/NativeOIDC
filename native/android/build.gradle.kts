import org.gradle.plugins.signing.Sign

plugins {
    id("com.android.library") version "8.11.1"
    id("org.jetbrains.kotlin.android") version "2.2.20"
    id("com.vanniktech.maven.publish") version "0.34.0"
}

val repositoryRoot = rootDir.resolve("../..").canonicalFile
// Release runs pass -PreleaseVersion=X.Y.Z. The SNAPSHOT fallback is rejected by
// Maven Central, so a publish without an explicit version cannot succeed.
val releaseVersion = providers.gradleProperty("releaseVersion").getOrElse("0.0.0-SNAPSHOT")
val generatedBindings = layout.buildDirectory.dir("generated/uniffi")
val androidRustLibraries = layout.buildDirectory.dir("generated/rust/jniLibs")
val generateUniFfiBindings by tasks.registering(Exec::class) {
    workingDir(repositoryRoot)
    commandLine(
        "bash",
        repositoryRoot.resolve("scripts/generate-bindings.sh").absolutePath,
        generatedBindings.get().asFile.absolutePath,
    )
    inputs.files(
        repositoryRoot.resolve("Cargo.toml"),
        repositoryRoot.resolve("Cargo.lock"),
        repositoryRoot.resolve("scripts/generate-bindings.sh"),
        fileTree(repositoryRoot.resolve("src")),
    )
    outputs.dir(generatedBindings)
}

// The Rust libraries and the standard library notices both come from the
// active toolchain, so a different Rust version must rebuild both together.
val rustToolchain = providers.exec {
    workingDir(repositoryRoot)
    commandLine("rustc", "--version", "--verbose")
}.standardOutput.asText

val buildAndroidRustLibraries by tasks.registering(Exec::class) {
    workingDir(repositoryRoot)
    commandLine(
        "bash",
        repositoryRoot.resolve("scripts/check-android-targets.sh").absolutePath,
        androidRustLibraries.get().asFile.absolutePath,
    )
    inputs.files(
        repositoryRoot.resolve("Cargo.toml"),
        repositoryRoot.resolve("Cargo.lock"),
        repositoryRoot.resolve("scripts/check-android-targets.sh"),
        fileTree(repositoryRoot.resolve("src")),
    )
    inputs.property("rustToolchain", rustToolchain)
    outputs.dir(androidRustLibraries)
}

// License notices for the release AAR, in classes.jar under a path of their
// own so they do not collide with other libraries' META-INF/LICENSE in apps.
// Only release builds ship them, so debug builds do not need cargo-about.
// The standard library notices come from the Rust toolchain running this
// build, which is also the one that builds the Rust libraries.
val generatedLicenses = layout.buildDirectory.dir("generated/licenses")
val generateLicenseNotices by tasks.registering(Exec::class) {
    val noticeDir = generatedLicenses.map { it.dir("META-INF/native-oidc").asFile }
    workingDir(repositoryRoot)
    commandLine(
        "bash",
        repositoryRoot.resolve("scripts/generate-third-party-licenses.sh").absolutePath,
        noticeDir.get().resolve("THIRD_PARTY_LICENSES.txt").absolutePath,
    )
    inputs.files(
        repositoryRoot.resolve("Cargo.toml"),
        repositoryRoot.resolve("Cargo.lock"),
        repositoryRoot.resolve("LICENSE"),
        repositoryRoot.resolve("about.toml"),
        repositoryRoot.resolve("about.hbs"),
        repositoryRoot.resolve("scripts/generate-third-party-licenses.sh"),
    )
    inputs.property("rustToolchain", rustToolchain)
    outputs.dir(generatedLicenses)
    doLast {
        repositoryRoot.resolve("LICENSE").copyTo(noticeDir.get().resolve("LICENSE"), overwrite = true)
    }
}

android {
    namespace = "net.projectflora.nativeoidc"
    compileSdk = 35

    defaultConfig {
        minSdk = 28
        testInstrumentationRunner = "androidx.test.runner.AndroidJUnitRunner"
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    kotlinOptions {
        jvmTarget = "17"
    }

    sourceSets.getByName("main").java.srcDir(generatedBindings.map { it.dir("kotlin") })
    sourceSets.getByName("main").jniLibs.srcDir(androidRustLibraries)
    sourceSets.getByName("release").resources.srcDir(generatedLicenses)

    testOptions {
        unitTests.isReturnDefaultValues = true
    }
}

tasks.named("preBuild").configure {
    dependsOn(generateUniFfiBindings)
}

tasks.configureEach {
    if (name == "mergeDebugJniLibFolders" || name == "mergeReleaseJniLibFolders") {
        dependsOn(buildAndroidRustLibraries)
    }
    // AGP does not infer this dependency from the resources source directory.
    if (name == "processReleaseJavaRes") {
        dependsOn(generateLicenseNotices)
    }
    // The published sources and javadoc jars read the generated bindings.
    if (name == "sourceReleaseJar" || name.startsWith("javaDocRelease")) {
        dependsOn(generateUniFfiBindings)
    }
}

// The Maven Central publication of a release adopts the AAR already attached
// to the GitHub Release (-PadoptedAar=<path>) instead of building a new one,
// so both channels ship the same bytes and a resumed publication needs no
// rebuild (Issue #180). The caller excludes the AAR build with
// -x bundleReleaseAar, and Gradle then also drops the tasks only that build
// needs: the Rust NDK builds and the license notices. The adopted file is
// copied to the path the AAR build would have written, so the publication,
// the hashes in its module metadata, and its signature all describe the
// adopted bytes.
val adoptedAar = providers.gradleProperty("adoptedAar").orNull?.let { file(it) }
if (adoptedAar != null) {
    val adoptReleaseAar by tasks.registering {
        inputs.file(adoptedAar)
        doLast {
            val bundle = tasks.getByName("bundleReleaseAar") as Zip
            adoptedAar.copyTo(bundle.archiveFile.get().asFile, overwrite = true)
        }
    }
    val publicationTask = Regex("^(generateMetadataFileFor.+Publication|sign.+Publication|publish.+PublicationTo.+)$")
    tasks.configureEach {
        if (publicationTask.matches(name)) {
            dependsOn(adoptReleaseAar)
        }
    }
    gradle.taskGraph.whenReady {
        val built = listOf("bundleReleaseAar", "buildAndroidRustLibraries", "generateLicenseNotices")
            .filter { name -> allTasks.any { it.project == project && it.name == name } }
        check(built.isEmpty()) {
            "-PadoptedAar must replace the AAR build, but the task graph still runs $built; " +
                "exclude it with -x bundleReleaseAar"
        }
    }
}

// Release runs stage the signed Maven Central bundle in a file repository
// (-PcentralBundleRepository=<dir>). Publishing to it writes the .md5 and
// .sha1 checksums Central requires next to every file and signature.
providers.gradleProperty("centralBundleRepository").orNull?.let { directory ->
    extensions.configure<PublishingExtension>("publishing") {
        repositories {
            maven {
                name = "centralBundle"
                url = uri(file(directory))
            }
        }
    }
}

dependencies {
    api("net.java.dev.jna:jna:5.12.0@aar")
    api("org.jetbrains.kotlinx:kotlinx-coroutines-android:1.10.2")
    implementation("androidx.browser:browser:1.8.0")

    androidTestImplementation("androidx.test.ext:junit:1.2.1")
    androidTestImplementation("androidx.test:runner:1.6.2")
    androidTestImplementation("androidx.test.uiautomator:uiautomator:2.3.0")

    testImplementation("junit:junit:4.13.2")
}

mavenPublishing {
    coordinates("net.project-flora.nativeoidc", "native-oidc-android", releaseVersion)
    publishToMavenCentral()
    // The release workflow supplies the key as ORG_GRADLE_PROJECT_signingInMemoryKey.
    // Without it (local builds, CI checks) publications are left unsigned;
    // Maven Central rejects unsigned uploads.
    if (providers.gradleProperty("signingInMemoryKey").isPresent) {
        signAllPublications()
    }

    pom {
        name.set("NativeOIDC Android")
        description.set("Android adapter and UniFFI Kotlin bindings for NativeOIDC, a Rust OAuth 2.0 and OpenID Connect client for native apps. NativeOIDC is an independent project by ProjectFlora. It is not affiliated with, endorsed by, or sponsored by the OpenID Foundation or the AppAuth projects (AppAuth-iOS, AppAuth-Android). OpenID is a trademark of the OpenID Foundation.")
        url.set("https://github.com/ProjectFlora/NativeOIDC")
        licenses {
            license {
                name.set("The Apache License, Version 2.0")
                url.set("https://www.apache.org/licenses/LICENSE-2.0.txt")
                distribution.set("repo")
            }
        }
        developers {
            developer {
                id.set("projectflora")
                name.set("ProjectFlora")
                url.set("https://github.com/ProjectFlora")
            }
        }
        scm {
            url.set("https://github.com/ProjectFlora/NativeOIDC")
            connection.set("scm:git:https://github.com/ProjectFlora/NativeOIDC.git")
            developerConnection.set("scm:git:ssh://git@github.com/ProjectFlora/NativeOIDC.git")
        }
    }
}

// Gradle's signing plugin signs with the bcpg jar in the Gradle
// distribution, whose armor "Version:" header still carries Bouncy
// Castle's unreplaced release-name placeholder ("Version: BCPG
// v@RELEASE_NAME@", Issue #169). Armor headers are comments outside the
// signed data, so the build drops the header from every generated
// signature: the published .asc then never carries a build-system
// leftover, whatever the Gradle or bcpg version. The publish tasks deploy
// the .asc files after the sign tasks and compute their checksums from the
// cleaned bytes, so no checksum adjustment is needed. Files without a
// "Version:" header are left byte-identical.
fun File.dropArmorVersionHeader() {
    val text = readText()
    val newline = if (text.contains("\r\n")) "\r\n" else "\n"
    var inHeader = false
    var changed = false
    val lines = text.split('\n').map { line ->
        val content = line.trimEnd('\r')
        if (content.startsWith("-----BEGIN PGP")) {
            inHeader = true
        } else if (inHeader && content.isEmpty()) {
            inHeader = false
        }
        if (inHeader && content.startsWith("Version:")) {
            changed = true
            null
        } else {
            content
        }
    }.filterNotNull()
    if (changed) {
        writeText(lines.joinToString(newline))
    }
}

tasks.withType(Sign::class.java).configureEach {
    doLast {
        signatures.forEach { signature ->
            val asc = signature.file
            if (asc.exists()) {
                asc.dropArmorVersionHeader()
            }
        }
    }
}
