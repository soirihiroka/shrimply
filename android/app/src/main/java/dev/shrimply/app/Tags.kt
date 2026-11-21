package dev.shrimply.app

import androidx.compose.foundation.layout.*
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.unit.dp
import org.json.JSONObject

/** Native tag entry and pills forward Rust's opaque commands. */
@Composable
internal fun Tags(control: JSONObject, enabled: Boolean, send: (JSONObject) -> Unit) {
    var draft by rememberSaveable(control.getString("id")) { mutableStateOf("") }
    fun add() { send(JSONObject(control.getJSONObject("add_command").toString()).put("value", draft)); draft = "" }
    Column(Modifier.fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(6.dp)) {
        OutlinedTextField(draft, { draft = it }, enabled = enabled, singleLine = true,
            label = { Text(control.getString("label")) }, placeholder = { Text(control.getString("placeholder")) },
            keyboardOptions = KeyboardOptions(imeAction = ImeAction.Done), keyboardActions = KeyboardActions(onDone = { add() }),
            trailingIcon = { IconButton(onClick = { add() }, enabled = enabled) {
                Icon(painterResource(inspectorIcon(control.getString("add_icon"))), control.getString("add_label"))
            } }, modifier = Modifier.fillMaxWidth())
        FlowRow(horizontalArrangement = Arrangement.spacedBy(6.dp)) {
            control.getJSONArray("tags").objects().forEach { tag -> key(tag.getString("value")) {
                InputChip(selected = false, onClick = { send(tag.getJSONObject("command")) }, enabled = enabled,
                    label = { Text(tag.getString("value")) }, trailingIcon = {
                        Icon(painterResource(inspectorIcon(control.getString("remove_icon"))), tag.getString("remove_label"), Modifier.size(16.dp))
                    })
            } }
        }
    }
}
