// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

import com.google.protobuf.gradle.id

plugins {
    kotlin("jvm")
    alias(libs.plugins.protobuf)
}

kotlin {
    jvmToolchain(21)
    compilerOptions {
        jvmTarget.set(org.jetbrains.kotlin.gradle.dsl.JvmTarget.JVM_17)
    }
}

java {
    sourceCompatibility = JavaVersion.VERSION_17
    targetCompatibility = JavaVersion.VERSION_17
}

sourceSets {
    main {
        proto {
            srcDir("../../../proto")
        }
    }
}

dependencies {
    api(libs.protobuf.javalite)
    api(libs.protobuf.kotlin.lite)
    api(libs.grpc.protobuf.lite)
    api(libs.grpc.stub)
    api(libs.grpc.kotlin.stub)
    api(libs.kotlinx.coroutines.core)
    // @Generated annotation used by generated grpc-java code.
    compileOnly("org.apache.tomcat:annotations-api:6.0.53")
}

fun Provider<MinimalExternalModuleDependency>.coordinates(classifier: String = ""): String =
    get().let { "${it.module}:${it.versionConstraint.requiredVersion}$classifier" }

protobuf {
    protoc {
        artifact = libs.protobuf.protoc.coordinates()
    }
    plugins {
        id("grpc") {
            artifact = libs.protoc.gen.grpc.java.coordinates()
        }
        id("grpckt") {
            artifact = libs.protoc.gen.grpc.kotlin.coordinates(":jdk8@jar")
        }
    }
    generateProtoTasks {
        all().forEach { task ->
            task.builtins {
                // "java" is a default builtin; configure it rather than adding it again.
                maybeCreate("java").option("lite")
                maybeCreate("kotlin").option("lite")
            }
            task.plugins {
                id("grpc") { option("lite") }
                id("grpckt") { option("lite") }
            }
        }
    }
}
