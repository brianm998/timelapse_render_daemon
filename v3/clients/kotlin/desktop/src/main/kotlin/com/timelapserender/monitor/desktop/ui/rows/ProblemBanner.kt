// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Timelapse Render Daemon contributors

package com.timelapserender.monitor.desktop.ui.rows

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import com.timelapserender.monitor.core.state.ProblemRowUi
import com.timelapserender.trd.v1.Severity

/** One problem (overview §6.1): red for an error, amber for a warning, with its title, detail and fix. */
@Composable
fun ProblemBanner(problem: ProblemRowUi, modifier: Modifier = Modifier) {
    val error = problem.severity == Severity.SEVERITY_ERROR
    Surface(
        color = if (error) ERROR_RED else WARNING_AMBER,
        contentColor = Color.White,
        shape = MaterialTheme.shapes.medium,
        modifier = modifier.fillMaxWidth().semantics(mergeDescendants = true) {
            contentDescription = if (error) "Error" else "Warning"
        },
    ) {
        Column(Modifier.padding(12.dp), verticalArrangement = Arrangement.spacedBy(2.dp)) {
            Text(problem.title, fontWeight = FontWeight.Bold)
            if (problem.detail.isNotEmpty()) Text(problem.detail)
            if (problem.fix.isNotEmpty()) Text(problem.fix, style = MaterialTheme.typography.bodySmall)
        }
    }
}
