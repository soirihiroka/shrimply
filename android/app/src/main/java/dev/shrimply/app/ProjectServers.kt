package dev.shrimply.app

import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import org.json.JSONObject

@Composable
internal fun ProjectServersDialog(servers: List<String>, busy: Boolean, close: () -> Unit,
    update: (JSONObject, () -> Unit) -> Unit) {
    var editing by rememberSaveable { mutableStateOf<String?>(null) }
    val previous = editing
    if (previous == null) {
        AlertDialog(onDismissRequest = { if (!busy) close() }, title = { Text(tr("Project Servers")) },
            text = {
                Column(Modifier.verticalScroll(rememberScrollState())) {
                    servers.forEach { server ->
                        ListItem(headlineContent = { Text(server, maxLines = 1, overflow = TextOverflow.Ellipsis) },
                            trailingContent = {
                                IconButton(onClick = { editing = server }, enabled = !busy) {
                                    Icon(iconVector("pencil-symbolic"), tr("Edit"), Modifier.size(16.dp))
                                }
                            })
                    }
                }
            },
            confirmButton = { TextButton(onClick = { editing = "" }, enabled = !busy) { Text(tr("Add")) } },
            dismissButton = { TextButton(onClick = close, enabled = !busy) { Text(tr("Close")) } })
        return
    }

    var url by rememberSaveable(previous) { mutableStateOf(previous) }
    var username by rememberSaveable(previous) { mutableStateOf("") }
    var password by remember(previous) { mutableStateOf("") }
    val command = { JSONObject().put("op", "update_project_server").apply {
        if (previous.isNotEmpty()) put("previous", previous)
    } }
    AlertDialog(modifier = Modifier.imePadding(), onDismissRequest = { if (!busy) editing = null },
        title = { Text(tr(if (previous.isEmpty()) "Add Project Server" else "Edit Project Server")) },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                OutlinedTextField(url, { url = it }, label = { Text(tr("Server URL")) }, singleLine = true, enabled = !busy)
                OutlinedTextField(username, { username = it }, label = { Text(tr("Username")) }, singleLine = true, enabled = !busy)
                OutlinedTextField(password, { password = it }, label = { Text(tr("Password")) }, singleLine = true,
                    visualTransformation = PasswordVisualTransformation(), enabled = !busy)
            }
        },
        confirmButton = { TextButton(enabled = !busy && url.isNotBlank() &&
            username.isNotBlank() && password.isNotEmpty(), onClick = {
            val request = command().put("server", url).put("username", username).put("password", password)
            password = ""
            update(request) { editing = null }
        }) { Text(tr("Sign In")) } },
        dismissButton = {
            if (previous.isNotEmpty()) TextButton(enabled = !busy, onClick = {
                update(command()) { editing = null }
            }) { Text(tr("Remove")) }
            TextButton(enabled = !busy, onClick = { editing = null }) { Text(tr("Cancel")) }
        })
}
