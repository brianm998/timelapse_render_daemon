// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

package com.timelapserender.monitor.desktop.ui.rows

import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.Image
import androidx.compose.foundation.TooltipArea
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.selection.selectable
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.toComposeImageBitmap
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.dp
import com.timelapserender.monitor.core.net.ConnectionState
import com.timelapserender.monitor.core.net.TrdClient
import com.timelapserender.monitor.core.state.RenderedRowUi
import com.timelapserender.monitor.core.state.ThumbnailCache
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import org.jetbrains.skia.Image as SkiaImage
import java.nio.file.Path

const val POSTER_TAG = "poster"
const val POSTER_PLACEHOLDER_TAG = "poster-placeholder"

/** The decoded poster of a rendered video, or null when there is none to show. */
fun interface PosterLoader {
    suspend fun load(renderedVideoId: String): ImageBitmap?
}

/**
 * Posters through a [ThumbnailCache] under [cacheDir], one cache per server id as the server reports
 * it (a sandbox `trd` and the installed one both are the "local" entry), decoded by Skia. A poster
 * that cannot be fetched or decoded is [report]ed and left out; a video the server has no poster for
 * is not a failure.
 */
class ThumbnailPosters(
    private val client: TrdClient,
    private val cacheDir: Path,
    private val report: (String) -> Unit = System.err::println,
) : PosterLoader {
    private val caches = HashMap<String, ThumbnailCache>()

    override suspend fun load(renderedVideoId: String): ImageBitmap? {
        val serverId = (client.state.value as? ConnectionState.Connected)?.info?.serverId ?: return null
        val cache = synchronized(caches) { caches.getOrPut(serverId) { ThumbnailCache(client, serverId, cacheDir) } }
        return withContext(Dispatchers.IO) {
            val bytes = try {
                cache.get(renderedVideoId)
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                report("Poster of video $renderedVideoId could not be fetched: $e")
                null
            } ?: return@withContext null
            try {
                SkiaImage.makeFromEncoded(bytes).toComposeImageBitmap()
            } catch (e: Exception) {
                // Not kept, so that the next request fetches it again.
                cache.discard(renderedVideoId)
                report("Poster of video $renderedVideoId could not be decoded: $e")
                null
            }
        }
    }
}

/** One recently rendered video (overview §6.1). */
@OptIn(ExperimentalFoundationApi::class)
@Composable
fun RenderedRow(
    video: RenderedRowUi,
    posters: PosterLoader,
    connected: Boolean,
    selected: Boolean,
    onSelect: () -> Unit,
    modifier: Modifier = Modifier,
) {
    Surface(
        color = rowColor(selected),
        shape = MaterialTheme.shapes.medium,
        modifier = modifier.fillMaxWidth().selectable(selected, onClick = onSelect),
    ) {
        Row(Modifier.padding(8.dp), horizontalArrangement = Arrangement.spacedBy(10.dp)) {
            Poster(video, posters, connected)
            Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(2.dp)) {
                MiddleEllipsisText(video.outputFilename, Modifier.fillMaxWidth())
                Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                    Text(video.variantLabel)
                    Text(video.sizeText)
                    Text(video.framesText)
                    Text(video.durationText)
                    TooltipArea(
                        tooltip = {
                            Surface(shadowElevation = 4.dp, shape = MaterialTheme.shapes.small) {
                                Text(video.absoluteTimeText, Modifier.padding(horizontal = 8.dp, vertical = 4.dp))
                            }
                        },
                    ) {
                        Text(video.relativeTimeText)
                    }
                }
                SecondaryText(video.exportPath)
                video.missingText?.let { SecondaryText(it) }
            }
        }
    }
}

/** A 96 dp wide 16:9 box: the poster letterboxed in it, grey while loading or missing. */
@Composable
private fun Poster(video: RenderedRowUi, posters: PosterLoader, connected: Boolean) {
    var image by remember(video.key) { mutableStateOf<ImageBitmap?>(null) }
    // Asked again on a reconnect, for a poster that could not be fetched.
    LaunchedEffect(video.key, video.hasThumbnail, posters, connected) {
        if (video.hasThumbnail) posters.load(video.key)?.let { image = it }
    }
    Box(Modifier.width(96.dp).aspectRatio(16f / 9f), contentAlignment = Alignment.Center) {
        val shown = image
        if (shown == null) {
            Box(Modifier.fillMaxSize().background(Color(0xFF9E9E9E)).testTag(POSTER_PLACEHOLDER_TAG))
        } else {
            Image(
                shown,
                contentDescription = "Poster",
                modifier = Modifier.fillMaxSize().background(Color.Black).testTag(POSTER_TAG),
                contentScale = ContentScale.Fit,
            )
        }
    }
}
