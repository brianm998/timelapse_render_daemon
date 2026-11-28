// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

package com.timelapserender.monitor.desktop.platform

import com.timelapserender.monitor.desktop.RecordingRunner
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import java.nio.file.Files
import java.nio.file.Paths
import java.util.concurrent.TimeUnit

class TrdServiceTest {
    private val runner = RecordingRunner()

    private fun service(os: HostOs, env: Map<String, String> = emptyMap()) =
        TrdService(os, runner, "/home/u") { env[it] }

    @Test
    fun theOsIsReadFromItsName() {
        assertEquals(HostOs.MAC, HostOs.of("Mac OS X"))
        assertEquals(HostOs.MAC, HostOs.of("Darwin"))
        assertEquals(HostOs.WINDOWS, HostOs.of("Windows 11"))
        assertEquals(HostOs.LINUX, HostOs.of("Linux"))
    }

    @Test
    fun aMissingUserIdStopsTheMacStart() {
        runner.results = mapOf("/usr/bin/id" to CommandResult(1, "id: failed"))
        assertEquals("id: failed", service(HostOs.MAC).start().error)
        assertEquals(1, runner.calls.size)
    }

    @Test
    fun aSilentFailureNamesTheExitCode() {
        runner.results = mapOf("schtasks" to CommandResult(2, ""))
        assertEquals("The command failed with exit code 2.", service(HostOs.WINDOWS).start().error)
    }

    @Test
    fun logFoldersFollowEachOs() {
        assertEquals(Paths.get("/home/u", "Library", "Logs", "TimelapseRenderDaemon"), service(HostOs.MAC).logFolder())
        assertEquals(Paths.get("/home/u", ".local", "state", "trd"), service(HostOs.LINUX).logFolder())
        assertEquals(
            Paths.get("/state", "trd"),
            service(HostOs.LINUX, mapOf("XDG_STATE_HOME" to "/state")).logFolder(),
        )
        assertEquals(
            Paths.get("/home/u", "AppData", "Local", "TimelapseRenderDaemon", "logs"),
            service(HostOs.WINDOWS).logFolder(),
        )
        assertEquals(
            Paths.get("/local", "TimelapseRenderDaemon", "logs"),
            service(HostOs.WINDOWS, mapOf("LOCALAPPDATA" to "/local")).logFolder(),
        )
    }

    @Test
    fun openingUsesEachOsProgram() {
        for ((os, program) in listOf(HostOs.MAC to "open", HostOs.LINUX to "xdg-open", HostOs.WINDOWS to "explorer.exe")) {
            runner.calls.clear()
            val service = service(os)
            assertTrue(service.openLogFolder().ok)
            assertTrue(service.openReleasesPage().ok)
            assertEquals(
                listOf(listOf(program, service.logFolder().toString()), listOf(program, RELEASES_URL)),
                runner.calls.toList(),
            )
        }
    }

    @Test
    fun explorerExitCodeOneIsSuccessButNotElsewhere() {
        runner.results = mapOf("explorer.exe" to CommandResult(1, ""), "xdg-open" to CommandResult(1, "no browser"))
        assertTrue(service(HostOs.WINDOWS).openReleasesPage().ok)
        assertEquals("no browser", service(HostOs.LINUX).openReleasesPage().error)
    }

    // The real runner is exercised with this JVM's own launcher only: never a service manager.
    private val java = Paths.get(System.getProperty("java.home"), "bin", "java").toString()

    @Test
    fun theSystemRunnerReturnsTheExitCodeAndOutput() {
        val ok = SystemCommandRunner(120).run(listOf(java, "-version"))
        assertEquals(0, ok.exitCode)
        assertTrue(ok.output, ok.output.contains("version"))
        val bad = SystemCommandRunner(120).run(listOf(java, "--no-such-option"))
        assertTrue(bad.exitCode > 0)
    }

    @Test
    fun theSystemRunnerReportsAProgramThatCannotStart() {
        val result = SystemCommandRunner(120).run(listOf("/no/such/program"))
        assertEquals(SystemCommandRunner.NOT_RUN, result.exitCode)
        assertTrue(result.output.isNotEmpty())
    }

    @Test
    fun theSystemRunnerGivesUpOnACommandThatDoesNotFinish() {
        val dir = Files.createTempDirectory("trm-runner")
        val source = dir.resolve("Wait.java")
        Files.write(
            source,
            "public class Wait { public static void main(String[] a) throws Exception { Thread.sleep(600000); } }"
                .toByteArray(),
        )
        try {
            val result = SystemCommandRunner(1).run(listOf(java, source.toString()))
            assertEquals(SystemCommandRunner.NOT_RUN, result.exitCode)
            assertTrue(result.output, result.output.contains("did not finish"))
        } finally {
            Files.delete(source)
            Files.delete(dir)
        }
    }

    @Test
    fun theSystemRunnerLeavesAnOpenerRunning() {
        val dir = Files.createTempDirectory("trm-opener")
        val source = dir.resolve("Later.java")
        val marker = dir.resolve("survived")
        Files.write(
            source,
            (
                "public class Later { public static void main(String[] a) throws Exception { " +
                    "Thread.sleep(5000); new java.io.File(a[0]).createNewFile(); } }"
                ).toByteArray(),
        )
        try {
            val result = SystemCommandRunner(1).runOpener(listOf(java, source.toString(), marker.toString()))
            assertEquals(CommandResult(0, ""), result)
            // The marker appears only if the program was not killed; the deadline guards a hang.
            val deadline = System.nanoTime() + TimeUnit.SECONDS.toNanos(120)
            while (!Files.exists(marker) && System.nanoTime() < deadline) Thread.sleep(50)
            assertTrue(Files.exists(marker))
        } finally {
            Files.deleteIfExists(marker)
            Files.delete(source)
            Files.delete(dir)
        }
    }
}
