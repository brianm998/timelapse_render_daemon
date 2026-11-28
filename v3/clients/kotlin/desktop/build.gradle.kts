// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

import org.jetbrains.compose.desktop.application.dsl.TargetFormat

plugins {
    kotlin("jvm")
    alias(libs.plugins.kotlin.compose)
    alias(libs.plugins.compose.multiplatform)
    alias(libs.plugins.kover)
}

kotlin {
    jvmToolchain(21)
    compilerOptions {
        allWarningsAsErrors.set(true)
    }
}

dependencies {
    implementation(project(":core"))
    implementation(compose.desktop.currentOs)
    implementation(compose.material3)
    implementation(libs.kotlinx.coroutines.swing)

    testImplementation(testFixtures(project(":core")))
    testImplementation(libs.junit)
    testImplementation(libs.kotlinx.coroutines.test)
    @OptIn(org.jetbrains.compose.ExperimentalComposeLibrary::class)
    testImplementation(compose.uiTest)
    @OptIn(org.jetbrains.compose.ExperimentalComposeLibrary::class)
    testImplementation(compose.desktop.uiTestJUnit4)
}

// Packaging (spec 2 §9). -Ptrm.version names the release being built; without it the version is
// v3/VERSION. Installers take only numbers, so a pre-release suffix is left out.
val trmVersion: String = providers.gradleProperty("trm.version").orNull?.takeIf { it.isNotBlank() }
    ?: providers.fileContents(rootProject.layout.projectDirectory.file("../../VERSION")).asText.get().trim()
val iconDir = rootProject.layout.projectDirectory.dir("../../assets/icon")
val signIdentity: String? = providers.environmentVariable("TRM_SIGN_IDENTITY").orNull?.takeIf { it.isNotBlank() }
val jpackageJdk: String? = providers.environmentVariable("TRM_JPACKAGE_JDK").orNull?.takeIf { it.isNotBlank() }

compose.desktop {
    application {
        mainClass = "com.timelapserender.monitor.desktop.MainKt"
        // The Android Studio JBR used for builds has no jpackage.
        jpackageJdk?.let { javaHome = it }

        // ProGuard's default rules do not cover the reflection in grpc, protobuf-lite and jmdns.
        buildTypes.release.proguard {
            isEnabled.set(false)
        }

        nativeDistributions {
            targetFormats(TargetFormat.Dmg, TargetFormat.Deb, TargetFormat.Msi)
            packageName = "Timelapse Render Monitor"
            packageVersion = trmVersion.substringBefore('-')
            vendor = "Timelapse Render"
            description = "Watches and controls the Timelapse Render Daemon"
            // The JDK modules of the packaged runtime. jdk.crypto.ec holds the elliptic curves of
            // the pinned TLS connection to a server on the network.
            // The rest is what `jdeps --print-module-deps` reports for the runtime classpath, and
            // java.logging.
            modules(
                "java.compiler",
                "java.instrument",
                "java.logging",
                "java.naming",
                "java.prefs",
                "java.sql",
                "jdk.crypto.ec",
                "jdk.unsupported",
            )

            macOS {
                bundleID = "com.timelapserender.monitor"
                iconFile.set(iconDir.file("monitor.icns"))
                if (signIdentity != null) {
                    signing {
                        sign.set(true)
                        identity.set(signIdentity)
                    }
                    entitlementsFile.set(layout.projectDirectory.file("packaging/macos-entitlements.plist"))
                    runtimeEntitlementsFile.set(layout.projectDirectory.file("packaging/macos-entitlements.plist"))
                }
            }
            windows {
                iconFile.set(iconDir.file("monitor.ico"))
                menu = true
                shortcut = true
                // Never change this: Windows upgrades an installed version by it.
                upgradeUuid = "2F33F927-6B13-4A24-B78F-D9A5084D6DFE"
            }
            linux {
                iconFile.set(iconDir.file("monitor-512.png"))
                packageName = "timelapse-render-monitor"
            }
        }
    }
}

tasks.withType<Test>().configureEach {
    systemProperty("java.awt.headless", "true")
}

kover {
    reports {
        filters {
            excludes {
                classes("com.timelapserender.trd.v1.*", "*.testfixtures.*")
            }
        }
        // The line-coverage floor of overview §15.3.
        verify {
            rule {
                minBound(50)
            }
        }
    }
}
