// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

package com.timelapserender.monitor.desktop.ui.config

import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.assertTextEquals
import androidx.compose.ui.test.performClick
import com.timelapserender.trd.v1.ValidateConfigRequest
import io.grpc.Status
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class CodecEditorTest : ConfigTestBase() {
    private val names: List<String> get() = controller.editor.state.value.codecs.map { it.name }

    private fun showCodecs() {
        show()
        open(ConfigGroup.CODECS)
    }

    private val unknown = "codec \"h264\" is not in the codec table"

    private fun awaitMessage(message: String) {
        awaitTag(CodecTags.MESSAGE)
        awaitText(message)
        node(CodecTags.MESSAGE).assertTextEquals(message)
    }

    @Test
    fun removingACodecThatAVariantUsesIsRefusedWithTheServersMessage() {
        showCodecs()
        answer(error("/rendering/0/codec", unknown))
        click(CodecTags.remove("h264"))
        awaitMessage("/rendering/0/codec: $unknown")
        assertEquals(listOf("h264", "prores"), names)
        assertFalse(controller.dirty)
        node(ConfigTags.SAVE).assertIsNotEnabled()
        // The server was asked about the table without it.
        val asked = fake.requestsOf<ValidateConfigRequest>().map { request -> request.codecsList.map { it.name } }
        assertTrue(listOf("prores") in asked)
    }

    @Test
    fun renamingACodecThatAVariantUsesIsRefused() {
        showCodecs()
        answer(error("/rendering/0/codec", unknown))
        type(CodecTags.name("h264"), "avc")
        click(CodecTags.rename("h264"))
        awaitMessage("/rendering/0/codec: $unknown")
        assertEquals(listOf("h264", "prores"), names)
    }

    @Test
    fun anUnusedCodecIsRemovedAndSavedWithTheLoadedRevision() {
        showCodecs()
        // An error about another variant's codec does not block the removal.
        answer(error("/rendering/0/frame_rate", "frame_rate must be 1 to 240"))
        click(CodecTags.remove("prores"))
        await { names == listOf("h264") }
        settle()
        node(CodecTags.remove("h264")).assertIsNotEnabled()
        answer()
        type(CodecTags.field("h264", "filename.quality"), "best")
        settle()
        fake.updateCodecsResult = state(codecs = listOf(codec("h264")))
        node(ConfigTags.SAVE).performClick()
        await { !controller.editor.state.value.dirty }
        val request = codecUpdates().single()
        assertEquals("codecs1", request.expectedRevision)
        assertEquals(listOf("h264"), request.codecsList.map { it.name })
        assertTrue(configUpdates().isEmpty())
    }

    @Test
    fun aRefusalAtSaveShowsTheServersMessage() {
        showCodecs()
        click(CodecTags.remove("prores"))
        await { names == listOf("h264") }
        settle()
        val message = "config has errors: /rendering/0/codec: codec \"prores\" is not in the codec table"
        fake.failWith["UpdateCodecs"] = Status.INVALID_ARGUMENT.withDescription(message)
        node(ConfigTags.SAVE).performClick()
        awaitText(message)
        assertTrue(controller.dirty)
    }

    @Test
    fun aRemovalThatCannotBeCheckedIsNotMade() {
        showCodecs()
        fake.failWith["ValidateConfig"] = Status.UNAVAILABLE.withDescription("trd went away")
        click(CodecTags.remove("prores"))
        awaitMessage("trd went away")
        assertEquals(listOf("h264", "prores"), names)
    }

    @Test
    fun anUnusedCodecIsRenamed() {
        showCodecs()
        type(CodecTags.name("prores"), "ProRes2")
        click(CodecTags.rename("prores"))
        await { names == listOf("h264", "ProRes2") }
        compose.waitForIdle()
        node(CodecTags.name("ProRes2")).assertExists()
        assertTrue(controller.dirty)
    }

    @Test
    fun aTakenNameIsRefused() {
        showCodecs()
        type(CodecTags.name("prores"), "h264")
        click(CodecTags.rename("prores"))
        awaitMessage("A codec named \"h264\" already exists")
        type(CodecTags.NEW_NAME, "prores")
        click(CodecTags.ADD)
        awaitMessage("A codec named \"prores\" already exists")
        assertEquals(listOf("h264", "prores"), names)
    }

    @Test
    fun aCodecIsAddedAndItsFieldsEdited() {
        showCodecs()
        type(CodecTags.NEW_NAME, "av1")
        click(CodecTags.ADD)
        await { names == listOf("h264", "prores", "av1") }
        compose.waitForIdle()
        for ((field, value) in listOf(
            "ffmpeg.codec" to "libaom-av1",
            "ffmpeg.pixel_format" to "yuv420p10le",
            "ffmpeg.color_range" to "pc",
            "filename.pixel_format" to "10bit",
            "filename.codec" to "AV1",
            "filename.quality" to "high",
        )) {
            type(CodecTags.field("av1", field), value)
        }
        click(CodecTags.field("av1", "ffmpeg.has_profile"))
        type(CodecTags.field("av1", "ffmpeg.profile"), "main")
        click(CodecTags.addArgument("av1"))
        type(CodecTags.field("av1", "ffmpeg.extra_args.0"), "30")
        click(CodecTags.addArgument("av1"))
        type(CodecTags.field("av1", "ffmpeg.extra_args.1"), "-crf")
        click(ListTags.up("/video_codecs/av1/extra_args/1"))
        val added = controller.editor.state.value.codecs.last()
        assertEquals("libaom-av1", added.ffmpeg.codec)
        assertEquals("yuv420p10le", added.ffmpeg.pixelFormat)
        assertEquals("pc", added.ffmpeg.colorRange)
        assertTrue(added.ffmpeg.hasProfile)
        assertEquals("main", added.ffmpeg.profile)
        assertEquals(listOf("-crf", "30"), added.ffmpeg.extraArgsList)
        assertEquals("10bit", added.filename.pixelFormat)
        assertEquals("AV1", added.filename.codec)
        assertEquals("high", added.filename.quality)

        click(ListTags.remove("/video_codecs/av1/extra_args/0"))
        click(CodecTags.field("av1", "ffmpeg.has_profile"))
        val changed = controller.editor.state.value.codecs.last()
        assertEquals(listOf("30"), changed.ffmpeg.extraArgsList)
        assertFalse(changed.ffmpeg.hasProfile)
        assertEquals("", changed.ffmpeg.profile)
    }
}
