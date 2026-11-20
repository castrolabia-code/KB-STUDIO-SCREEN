package com.castrolabs.kbstudioscreen

import android.content.Intent
import android.net.Uri
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.lifecycle.lifecycleScope
import com.journeyapps.barcodescanner.ScanContract
import com.journeyapps.barcodescanner.ScanOptions
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import org.json.JSONObject

val Graphite = Color(0xFF15171A)
val Panel = Color(0xFF1F2226)
val Metal = Color(0xFF8A9099)
val TextHi = Color(0xFFE7E9EC)
val Orange = Color(0xFFFF7A1A)

private val KbColors = darkColorScheme(
    primary = Orange, onPrimary = Color(0xFF1A0E05), background = Graphite, surface = Panel,
    onSurface = TextHi, onBackground = TextHi, secondary = Metal, error = Color(0xFFFF6B6B),
)

class MainActivity : ComponentActivity() {
    private lateinit var storage: Storage
    private var discovery: Discovery? = null

    private val found = mutableStateOf<List<FoundHost>>(emptyList())
    private val saved = mutableStateOf<List<SavedHost>>(emptyList())
    private val busy = mutableStateOf<String?>(null)
    private val error = mutableStateOf<String?>(null)
    private val pinFor = mutableStateOf<PendingPin?>(null)

    data class PendingPin(val handle: Long, val host: String, val port: Int)

    private val scanQr = registerForActivityResult(ScanContract()) { r ->
        val uri = r.contents ?: return@registerForActivityResult
        val u = Uri.parse(uri)
        val host = u.getQueryParameter("h")
        val port = u.getQueryParameter("p")?.toIntOrNull()
        if (!uri.startsWith("kbss://pair?") || host == null || port == null) {
            error.value = getString(R.string.err_qr_invalid)
        } else {
            connect(host, port, null, uri)
        }
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        storage = Storage(this)
        saved.value = storage.hosts()
        setContent { MaterialTheme(colorScheme = KbColors) { Home() } }
    }

    override fun onResume() {
        super.onResume()
        saved.value = storage.hosts()
        discovery = Discovery(this) { list -> runOnUiThread { found.value = list } }.also { it.start() }
    }

    override fun onPause() {
        super.onPause()
        discovery?.stop()
        discovery = null
    }

    private fun connect(host: String, port: Int, fp: String?, qrUri: String?) {
        error.value = null
        busy.value = getString(R.string.connecting)
        lifecycleScope.launch {
            try {
                val (cert, key) = withContext(Dispatchers.Default) { storage.identity() }
                val known = storage.hosts().firstOrNull { (fp != null && it.fp == fp) || (fp == null && it.host == host && it.port == port) }
                val mode = when {
                    qrUri != null -> NativeCore.MODE_QR
                    known != null -> NativeCore.MODE_TRUSTED
                    else -> NativeCore.MODE_PIN
                }
                val info = DeviceInfo.detect(this@MainActivity)
                val h = withContext(Dispatchers.IO) {
                    NativeCore.connect(
                        host, port, mode, known?.fp ?: "", qrUri ?: "", cert, key, storage.deviceName, info.isTablet,
                        info.widthPx, info.heightPx, info.dpi, info.refresh, info.decMaxW, info.decMaxH,
                    )
                }
                val st = JSONObject(NativeCore.stage(h))
                if (st.getString("stage") == "need_pin") {
                    // La PC no nos reconoce (o es nueva): emparejar por PIN.
                    known?.let { storage.forget(it.fp) }
                    withContext(Dispatchers.IO) { NativeCore.requestPin(h) }
                    busy.value = null
                    pinFor.value = PendingPin(h, host, port)
                } else {
                    ready(h, host, port)
                }
            } catch (e: Throwable) {
                busy.value = null
                error.value = errorMessage(this@MainActivity, e)
            }
        }
    }

    private fun submitPin(p: PendingPin, pin: String) {
        pinFor.value = null
        busy.value = getString(R.string.waiting_approval)
        lifecycleScope.launch {
            try {
                withContext(Dispatchers.IO) { NativeCore.submitPin(p.handle, pin) }
                ready(p.handle, p.host, p.port)
            } catch (e: Throwable) {
                NativeCore.close(p.handle)
                busy.value = null
                error.value = errorMessage(this@MainActivity, e)
                // PIN incorrecto: se reconecta y la PC sigue mostrando el mismo PIN.
                if (e.message?.startsWith("pin_wrong") == true) connect(p.host, p.port, null, null)
            }
        }
    }

    private fun ready(h: Long, host: String, port: Int) {
        val hi = JSONObject(NativeCore.hostInfo(h))
        val sh = SavedHost(hi.getString("name"), host, port, hi.getString("fp"))
        storage.saveHost(sh)
        saved.value = storage.hosts()
        SessionHolder.handle = h
        SessionHolder.host = sh
        busy.value = null
        startActivity(Intent(this, ViewerActivity::class.java))
    }

    @Composable
    private fun Home() {
        var manual by remember { mutableStateOf(false) }
        var nameEdit by remember { mutableStateOf(false) }
        val wide = LocalConfiguration.current.screenWidthDp >= 700
        Box(
            Modifier.fillMaxSize().background(Brush.verticalGradient(listOf(Color(0xFF1B1E22), Graphite)))
                .systemBarsPadding().padding(horizontal = if (wide) 40.dp else 16.dp, vertical = 20.dp)
        ) {
            Column(Modifier.fillMaxSize()) {
                Header(onName = { nameEdit = true })
                Spacer(Modifier.height(20.dp))
                Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                    BigButton(stringResource(R.string.scan_qr), Icons.Filled.QrCodeScanner, Modifier.weight(1f)) {
                        scanQr.launch(ScanOptions().setPrompt(getString(R.string.qr_prompt)).setBeepEnabled(false).setOrientationLocked(false))
                    }
                    BigButton(stringResource(R.string.manual_ip), Icons.Filled.Lan, Modifier.weight(1f), outlined = true) { manual = true }
                }
                Spacer(Modifier.height(20.dp))
                error.value?.let { ErrorCard(it) { error.value = null } }
                busy.value?.let { BusyCard(it) }
                if (wide) {
                    Row(Modifier.fillMaxSize(), horizontalArrangement = Arrangement.spacedBy(20.dp)) {
                        Box(Modifier.weight(1f)) { FoundList() }
                        Box(Modifier.weight(1f)) { SavedList() }
                    }
                } else {
                    LazyColumn(verticalArrangement = Arrangement.spacedBy(10.dp)) {
                        item { FoundList() }
                        item { SavedList() }
                    }
                }
            }
        }
        if (manual) ManualDialog(onDismiss = { manual = false }) { h, p -> manual = false; connect(h, p, null, null) }
        if (nameEdit) NameDialog { nameEdit = false }
        pinFor.value?.let { p -> PinDialog(onCancel = { NativeCore.close(p.handle); pinFor.value = null }) { submitPin(p, it) } }
    }

    @Composable
    private fun Header(onName: () -> Unit) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Column(Modifier.weight(1f)) {
                Text("KB STUDIO SCREEN", color = TextHi, fontSize = 26.sp, fontWeight = FontWeight.Bold, letterSpacing = 1.5.sp)
                Text(stringResource(R.string.brand), color = Orange, fontSize = 14.sp)
            }
            TextButton(onClick = onName) {
                Icon(Icons.Filled.Tablet, null, tint = Metal)
                Spacer(Modifier.width(8.dp))
                Text(storage.deviceName, color = Metal, maxLines = 1)
            }
        }
    }

    @Composable
    private fun FoundList() {
        Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
            SectionTitle(stringResource(R.string.found_pcs))
            val savedFps = saved.value.map { it.fp }.toSet()
            if (found.value.isEmpty()) {
                Glass { Column(Modifier.padding(18.dp)) {
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        CircularProgressIndicator(Modifier.size(18.dp), strokeWidth = 2.dp, color = Orange)
                        Spacer(Modifier.width(12.dp))
                        Text(stringResource(R.string.searching), color = TextHi)
                    }
                    Spacer(Modifier.height(10.dp))
                    Text(stringResource(R.string.none_found), color = Metal, fontSize = 14.sp)
                } }
            }
            found.value.forEach { f ->
                val isSaved = f.fp != null && f.fp in savedFps
                HostRow(f.name, "${f.host}:${f.port}", if (isSaved) stringResource(R.string.paired) else stringResource(R.string.new_pc), isSaved) {
                    connect(f.host, f.port, f.fp, null)
                }
            }
        }
    }

    @Composable
    private fun SavedList() {
        Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
            if (saved.value.isNotEmpty()) SectionTitle(stringResource(R.string.saved_pcs))
            saved.value.forEach { s ->
                HostRow(s.name, "${s.host}:${s.port}", stringResource(R.string.paired), true, onForget = {
                    storage.forget(s.fp); saved.value = storage.hosts()
                }) { connect(s.host, s.port, s.fp, null) }
            }
            Text(stringResource(R.string.no_audio_note), color = Metal.copy(alpha = 0.7f), fontSize = 12.sp, modifier = Modifier.padding(top = 8.dp))
        }
    }

    @Composable
    private fun NameDialog(onDone: () -> Unit) {
        var v by remember { mutableStateOf(storage.deviceName) }
        AlertDialog(onDismissRequest = onDone, containerColor = Panel,
            title = { Text(stringResource(R.string.device_name)) },
            text = { OutlinedTextField(v, { v = it.take(40) }, singleLine = true) },
            confirmButton = { TextButton({ if (v.isNotBlank()) storage.deviceName = v.trim(); onDone() }) { Text("OK") } },
            dismissButton = { TextButton(onDone) { Text(stringResource(R.string.cancel)) } })
    }
}

@Composable
fun Glass(modifier: Modifier = Modifier, content: @Composable () -> Unit) {
    // Glassmorphism moderado: panel translúcido con borde metálico sutil, sin perder contraste.
    Surface(
        modifier = modifier.fillMaxWidth(), shape = RoundedCornerShape(16.dp),
        color = Panel.copy(alpha = 0.82f), border = BorderStroke(1.dp, Color.White.copy(alpha = 0.08f)),
    ) { content() }
}

@Composable
fun SectionTitle(t: String) = Text(t.uppercase(), color = Metal, fontSize = 13.sp, fontWeight = FontWeight.SemiBold, letterSpacing = 1.2.sp)

@Composable
fun BigButton(label: String, icon: androidx.compose.ui.graphics.vector.ImageVector, modifier: Modifier, outlined: Boolean = false, onClick: () -> Unit) {
    val content: @Composable RowScope.() -> Unit = {
        Icon(icon, null); Spacer(Modifier.width(10.dp)); Text(label, fontSize = 17.sp, fontWeight = FontWeight.SemiBold)
    }
    if (outlined) OutlinedButton(onClick, modifier.heightIn(min = 60.dp), shape = RoundedCornerShape(14.dp), border = BorderStroke(1.dp, Metal), content = content)
    else Button(onClick, modifier.heightIn(min = 60.dp), shape = RoundedCornerShape(14.dp), content = content)
}

@Composable
fun HostRow(name: String, addr: String, badge: String, paired: Boolean, onForget: (() -> Unit)? = null, onClick: () -> Unit) {
    Glass(Modifier.clickable(onClick = onClick)) {
        Row(Modifier.padding(horizontal = 18.dp, vertical = 16.dp).heightIn(min = 40.dp), verticalAlignment = Alignment.CenterVertically) {
            Icon(Icons.Filled.Computer, null, tint = if (paired) Orange else Metal, modifier = Modifier.size(30.dp))
            Spacer(Modifier.width(14.dp))
            Column(Modifier.weight(1f)) {
                Text(name, color = TextHi, fontSize = 18.sp, fontWeight = FontWeight.Medium)
                Text("$addr · $badge", color = Metal, fontSize = 13.sp, fontFamily = FontFamily.Monospace)
            }
            if (onForget != null) IconButton(onForget, Modifier.size(48.dp)) { Icon(Icons.Filled.DeleteOutline, null, tint = Metal) }
            Icon(Icons.Filled.ChevronRight, null, tint = Metal)
        }
    }
}

@Composable
fun ErrorCard(msg: String, onClose: () -> Unit) {
    Glass(Modifier.padding(bottom = 12.dp)) {
        Row(Modifier.padding(16.dp), verticalAlignment = Alignment.CenterVertically) {
            Icon(Icons.Filled.ErrorOutline, null, tint = Color(0xFFFF6B6B))
            Spacer(Modifier.width(12.dp))
            Text(msg, color = TextHi, modifier = Modifier.weight(1f))
            IconButton(onClose) { Icon(Icons.Filled.Close, null, tint = Metal) }
        }
    }
}

@Composable
fun BusyCard(msg: String) {
    Glass(Modifier.padding(bottom = 12.dp)) {
        Row(Modifier.padding(18.dp), verticalAlignment = Alignment.CenterVertically) {
            CircularProgressIndicator(Modifier.size(20.dp), strokeWidth = 2.dp, color = Orange)
            Spacer(Modifier.width(14.dp))
            Text(msg, color = TextHi)
        }
    }
}

@Composable
fun ManualDialog(onDismiss: () -> Unit, onOk: (String, Int) -> Unit) {
    var host by remember { mutableStateOf("") }
    var port by remember { mutableStateOf("47800") }
    AlertDialog(onDismissRequest = onDismiss, containerColor = Panel,
        title = { Text(stringResource(R.string.manual_ip)) },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
                OutlinedTextField(host, { host = it.trim() }, label = { Text(stringResource(R.string.host_ip)) }, singleLine = true,
                    keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Uri))
                OutlinedTextField(port, { port = it.filter(Char::isDigit).take(5) }, label = { Text(stringResource(R.string.port)) }, singleLine = true,
                    keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Number))
            }
        },
        confirmButton = { TextButton({ port.toIntOrNull()?.let { if (host.isNotEmpty()) onOk(host, it) } }) { Text(stringResource(R.string.connect)) } },
        dismissButton = { TextButton(onDismiss) { Text(stringResource(R.string.cancel)) } })
}

@Composable
fun PinDialog(onCancel: () -> Unit, onOk: (String) -> Unit) {
    var pin by remember { mutableStateOf("") }
    AlertDialog(onDismissRequest = {}, containerColor = Panel,
        title = { Text(stringResource(R.string.enter_pin_title)) },
        text = {
            Column {
                Text(stringResource(R.string.enter_pin_body), color = Metal)
                Spacer(Modifier.height(16.dp))
                OutlinedTextField(pin, { pin = it.filter(Char::isDigit).take(6) }, singleLine = true,
                    textStyle = LocalTextStyle.current.copy(fontSize = 32.sp, letterSpacing = 8.sp, textAlign = TextAlign.Center, fontFamily = FontFamily.Monospace),
                    keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.NumberPassword), modifier = Modifier.fillMaxWidth())
            }
        },
        confirmButton = { Button({ if (pin.length == 6) onOk(pin) }, enabled = pin.length == 6) { Text(stringResource(R.string.connect)) } },
        dismissButton = { TextButton(onCancel) { Text(stringResource(R.string.cancel)) } })
}
