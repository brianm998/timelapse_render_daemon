// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

package com.timelapserender.monitor.core.state

import com.timelapserender.trd.v1.DirEntry
import com.timelapserender.trd.v1.ListDirectoryRequest
import com.timelapserender.trd.v1.ListDirectoryResponse
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import kotlin.time.Duration.Companion.seconds

class DirectoryBrowserModelTest {
    private fun dir(name: String, path: String) = DirEntry.newBuilder().setName(name).setPath(path).setIsDir(true).build()

    private fun listing(path: String, parent: String, vararg entries: DirEntry) =
        ListDirectoryResponse.newBuilder().setPath(path).setParent(parent).addAllEntries(entries.toList())
            .addAllRoots(listOf("/", "/home/me")).build()

    private fun script(env: Env) {
        env.fake.directories = mapOf(
            "" to ListDirectoryResponse.newBuilder().addAllRoots(listOf("/", "/home/me")).build(),
            "/" to listing("/", "", dir("usr", "/usr"), dir("var", "/var")),
            "/usr" to listing("/usr", "/", dir("lib", "/usr/lib")),
        )
    }

    @Test
    fun showsTheRoots() = runTest(timeout = 60.seconds) {
        Env(backgroundScope).use { env ->
            script(env)
            val m = DirectoryBrowserModel(env.client, backgroundScope)
            m.openRoots()
            val ui = m.state.first { !it.loading && it.roots.isNotEmpty() }
            assertEquals(listOf("/", "/home/me"), ui.roots)
            assertTrue(ui.atRoots)
            assertNull(ui.selection)
        }
    }

    @Test
    fun navigatesIntoAFolderAndUpThroughTheParent() = runTest(timeout = 60.seconds) {
        Env(backgroundScope).use { env ->
            script(env)
            val m = DirectoryBrowserModel(env.client, backgroundScope, includeFiles = true, showHidden = true)
            m.navigate("/usr")
            val usr = m.state.first { it.path == "/usr" && !it.loading }
            assertEquals("/", usr.parent)
            assertEquals(listOf("/usr/lib"), usr.entries.map { it.path })
            val request = env.fake.requestsOf<ListDirectoryRequest>().single()
            assertTrue(request.includeFiles && request.showHidden)
            m.up()
            assertEquals(listOf("/usr", "/var"), m.state.first { it.path == "/" && !it.loading }.entries.map { it.path })
            m.up() // from a root folder up is the list of roots
            assertTrue(m.state.first { it.atRoots && !it.loading }.atRoots)
        }
    }

    @Test
    fun selectionDefaultsToTheCurrentFolderAndCanBeAnEntry() = runTest(timeout = 60.seconds) {
        Env(backgroundScope).use { env ->
            script(env)
            val m = DirectoryBrowserModel(env.client, backgroundScope)
            m.navigate("/")
            m.state.first { it.path == "/" && !it.loading }
            assertEquals("/", m.state.value.selection)
            m.select("/var")
            assertEquals("/var", m.state.value.selection)
            m.navigate("/usr")
            assertEquals("/usr", m.state.first { it.path == "/usr" && !it.loading }.selection)
        }
    }

    @Test
    fun aFailureKeepsTheListingAndReportsTheError() = runTest(timeout = 60.seconds) {
        Env(backgroundScope).use { env ->
            script(env)
            val m = DirectoryBrowserModel(env.client, backgroundScope)
            m.navigate("/")
            m.state.first { it.path == "/" && !it.loading }
            m.navigate("/nope")
            val ui = m.state.first { it.error != null }
            assertEquals("no such directory", ui.error)
            assertEquals("/", ui.path)
            assertEquals(2, ui.entries.size)
        }
    }
}
