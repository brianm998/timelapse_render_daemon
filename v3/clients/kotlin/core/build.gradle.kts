// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

plugins {
    kotlin("jvm")
    `java-test-fixtures`
    alias(libs.plugins.kover)
}

kotlin {
    jvmToolchain(21)
    compilerOptions {
        jvmTarget.set(org.jetbrains.kotlin.gradle.dsl.JvmTarget.JVM_17)
        allWarningsAsErrors.set(true)
    }
}

java {
    sourceCompatibility = JavaVersion.VERSION_17
    targetCompatibility = JavaVersion.VERSION_17
}

dependencies {
    api(project(":proto"))
    api(libs.kotlinx.coroutines.core)
    api(libs.grpc.okhttp)
    api(libs.jmdns)

    testFixturesApi(project(":proto"))
    testFixturesApi(libs.kotlinx.coroutines.core)
    testFixturesApi(libs.grpc.stub)
    testFixturesApi(libs.grpc.kotlin.stub)
    testFixturesApi(libs.grpc.inprocess)
    testFixturesApi(libs.grpc.netty.shaded)

    testImplementation(libs.junit)
    testImplementation(libs.kotlinx.coroutines.test)
    testImplementation(libs.grpc.inprocess)
    testImplementation(libs.grpc.testing)
}

kover {
    reports {
        filters {
            excludes {
                packages("com.timelapserender.trd.v1")
                classes("com.timelapserender.trd.v1.*", "*.testfixtures.*")
            }
        }
        verify {
            rule {
                minBound(0)
            }
        }
    }
}
