// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

pluginManagement {
    repositories {
        google()
        mavenCentral()
        gradlePluginPortal()
    }
}

dependencyResolutionManagement {
    repositoriesMode.set(RepositoriesMode.FAIL_ON_PROJECT_REPOS)
    repositories {
        google()
        mavenCentral()
    }
}

rootProject.name = "timelapse-render-monitor"

include(":proto", ":core", ":desktop")

// :android (spec 4) joins the build once its build file exists.
if (file("android/build.gradle.kts").exists()) {
    include(":android")
}
