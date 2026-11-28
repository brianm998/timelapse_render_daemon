// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

package com.timelapserender.monitor.desktop.platform

import java.io.IOException
import java.nio.file.Files
import java.nio.file.Path
import java.nio.file.Paths
import java.util.concurrent.TimeUnit

const val RELEASES_URL = "https://github.com/brianm998/timelapse_render_daemon/releases"

enum class HostOs {
    MAC, LINUX, WINDOWS;

    companion object {
        /** [osName] is `os.name`. */
        fun of(osName: String): HostOs {
            val os = osName.lowercase()
            return when {
                os.contains("mac") || os.contains("darwin") -> MAC
                os.contains("win") -> WINDOWS
                else -> LINUX
            }
        }

        val current: HostOs get() = of(System.getProperty("os.name").orEmpty())
    }
}

/** The exit code and the combined standard output and error of one command. */
data class CommandResult(val exitCode: Int, val output: String)

/** Runs one command. Tests record the calls instead of running anything. */
fun interface CommandRunner {
    fun run(argv: List<String>): CommandResult
}

/** Runs commands for real, giving up after [timeoutSeconds]. */
class SystemCommandRunner(private val timeoutSeconds: Long = 30) : CommandRunner {
    override fun run(argv: List<String>): CommandResult {
        // Output goes to a file so that a command that never exits cannot block the reader.
        val log = Files.createTempFile("trm-command", ".log")
        try {
            val process = try {
                ProcessBuilder(argv).redirectErrorStream(true).redirectOutput(log.toFile()).start()
            } catch (e: IOException) {
                return CommandResult(NOT_RUN, e.message ?: "Could not run ${argv.firstOrNull()}")
            }
            process.outputStream.close()
            val finished = process.waitFor(timeoutSeconds, TimeUnit.SECONDS)
            if (!finished) process.destroyForcibly()
            val output = String(Files.readAllBytes(log), Charsets.UTF_8).trim()
            return if (finished) {
                CommandResult(process.exitValue(), output)
            } else {
                CommandResult(NOT_RUN, "$output\nThe command did not finish in $timeoutSeconds s.".trim())
            }
        } finally {
            runCatching { Files.deleteIfExists(log) }
        }
    }

    companion object {
        /** The exit code of a command that could not be started or did not finish. */
        const val NOT_RUN = -1
    }
}

/** The outcome of an action on the local computer: [error] is ready to show, null on success. */
data class ActionResult(val error: String? = null) {
    val ok: Boolean get() = error == null
}

/**
 * What the app does to the computer it runs on: starting the installed `trd` (spec 2 §6.1, with
 * the spec 1 §10.4 service names), opening its log folder and opening web pages.
 */
class TrdService(
    private val os: HostOs,
    private val runner: CommandRunner,
    private val home: String,
    private val env: (String) -> String? = System::getenv,
) {
    /** The start command of this OS. [uid] is used on macOS only. */
    fun startCommand(uid: String): List<String> = when (os) {
        HostOs.MAC -> listOf("launchctl", "kickstart", "-k", "gui/$uid/$LAUNCHD_LABEL")
        HostOs.LINUX -> listOf("systemctl", "--user", "start", "trd")
        HostOs.WINDOWS -> listOf("schtasks", "/Run", "/TN", WINDOWS_TASK)
    }

    /** Runs the start command; a failure carries the command's output. Blocks. */
    fun start(): ActionResult {
        var uid = ""
        if (os == HostOs.MAC) {
            val id = runner.run(listOf("/usr/bin/id", "-u"))
            uid = id.output.trim()
            if (id.exitCode != 0 || uid.toLongOrNull() == null) return failure(id)
        }
        val result = runner.run(startCommand(uid))
        return if (result.exitCode == 0) ActionResult() else failure(result)
    }

    /** Where `trd` writes its log (overview §4). */
    fun logFolder(): Path = when (os) {
        HostOs.MAC -> Paths.get(home, "Library", "Logs", "TimelapseRenderDaemon")
        HostOs.LINUX -> Paths.get(
            env("XDG_STATE_HOME")?.takeIf { it.isNotEmpty() } ?: Paths.get(home, ".local", "state").toString(),
        ).resolve("trd")
        HostOs.WINDOWS -> Paths.get(
            env("LOCALAPPDATA")?.takeIf { it.isNotEmpty() } ?: Paths.get(home, "AppData", "Local").toString(),
        ).resolve("TimelapseRenderDaemon").resolve("logs")
    }

    fun openLogFolder(): ActionResult = open(logFolder().toString())

    fun openReleasesPage(): ActionResult = open(RELEASES_URL)

    private fun open(target: String): ActionResult {
        val program = when (os) {
            HostOs.MAC -> "open"
            HostOs.LINUX -> "xdg-open"
            HostOs.WINDOWS -> "explorer.exe"
        }
        val result = runner.run(listOf(program, target))
        // explorer.exe exits with 1 even when it opened the target.
        val ok = result.exitCode == 0 || (os == HostOs.WINDOWS && result.exitCode == 1)
        return if (ok) ActionResult() else failure(result)
    }

    private fun failure(result: CommandResult) =
        ActionResult(result.output.trim().ifEmpty { "The command failed with exit code ${result.exitCode}." })

    companion object {
        const val LAUNCHD_LABEL = "com.timelapserender.trd"
        const val WINDOWS_TASK = "TimelapseRenderDaemon"
    }
}
