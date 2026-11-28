// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

package com.timelapserender.monitor.desktop.platform

import java.nio.file.InvalidPathException
import java.nio.file.Paths

/**
 * Opens a rendered video of this computer's `trd`, or shows it in the file manager (spec 2 §6.4).
 * Both block until the command has answered.
 */
class FileOpener(private val os: HostOs, private val runner: CommandRunner) {
    fun openCommand(path: String): List<String> = listOf(program(), path)

    /** Linux file managers cannot be asked to select a file, so its directory is opened. */
    fun revealCommand(path: String): List<String> = when (os) {
        HostOs.MAC -> listOf(program(), "-R", path)
        HostOs.LINUX -> listOf(program(), directoryOf(path))
        HostOs.WINDOWS -> listOf(program(), "/select,$path")
    }

    fun open(path: String): ActionResult = run(openCommand(path))

    fun reveal(path: String): ActionResult = run(revealCommand(path))

    private fun program() = when (os) {
        HostOs.MAC -> "open"
        HostOs.LINUX -> "xdg-open"
        HostOs.WINDOWS -> "explorer.exe"
    }

    private fun directoryOf(path: String): String =
        try {
            Paths.get(path).parent?.toString() ?: path
        } catch (e: InvalidPathException) {
            path
        }

    private fun run(argv: List<String>): ActionResult = openerResult(os, runner.runOpener(argv))

    companion object {
        /** Open and Reveal are offered for a `trd` on this computer whose file still exists. */
        fun available(loopback: Boolean, fileExists: Boolean): Boolean = loopback && fileExists
    }
}

/** The outcome of a program that opened a file, a folder or a page. */
internal fun openerResult(os: HostOs, result: CommandResult): ActionResult {
    // explorer.exe exits with 1 even when it opened the target.
    val ok = result.exitCode == 0 || (os == HostOs.WINDOWS && result.exitCode == 1)
    return if (ok) ActionResult() else commandFailure(result)
}

internal fun commandFailure(result: CommandResult) =
    ActionResult(result.output.trim().ifEmpty { "The command failed with exit code ${result.exitCode}." })
