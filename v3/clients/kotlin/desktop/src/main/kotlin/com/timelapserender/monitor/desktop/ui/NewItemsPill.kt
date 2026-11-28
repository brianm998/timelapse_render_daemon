// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

package com.timelapserender.monitor.desktop.ui

import androidx.compose.foundation.layout.padding
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.dp

const val NEW_ITEMS_PILL_TAG = "new-items-pill"

/** The floating `↑ N new` pill of overview §6.2 S3; a click goes back to the top. */
@Composable
fun NewItemsPill(count: Int, onClick: () -> Unit, modifier: Modifier = Modifier) {
    Surface(
        onClick = onClick,
        color = MaterialTheme.colorScheme.primary,
        contentColor = MaterialTheme.colorScheme.onPrimary,
        shape = MaterialTheme.shapes.extraLarge,
        shadowElevation = 4.dp,
        modifier = modifier.testTag(NEW_ITEMS_PILL_TAG),
    ) {
        Text("↑ $count new", Modifier.padding(horizontal = 16.dp, vertical = 6.dp))
    }
}
