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
import androidx.compose.ui.text.font.Font
import androidx.compose.ui.text.font.FontStyle
import androidx.compose.ui.graphics.Shadow
import androidx.compose.ui.geometry.Offset
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.border
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.drawscope.clipRect
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

// Paleta "Vice nights": atardecer neón (arte y nombres propios, sin marcas de terceros).
val Graphite = Color(0xFF120A24)   // noche violeta
val Panel = Color(0xFF24123F)
val Metal = Color(0xFFC6B3D9)
val TextHi = Color(0xFFFFF4FB)
val Orange = Color(0xFFFF2E88)     // acento principal (rosa neón)
val SunOrange = Color(0xFFFF8A3D)
val Yellow = Color(0xFFFFD166)
val Cyan = Color(0xFF2EE6D6)
val Sunset = Brush.horizontalGradient(listOf(Color(0xFFFF2E88), Color(0xFFFF6A3D), Color(0xFFFFD166)))
/** Títulos: Anton (OFL), en itálica sintética para el aire de afiche. */
val Display = FontFamily(Font(R.font.anton))

private val KbColors = darkColorScheme(
    primary = Orange, onPrimary = Color(0xFF1A0716), background = Graphite, surface = Panel,
    onSurface = TextHi, onBackground = TextHi, secondary = Cyan, tertiary = Yellow, error = Color(0xFFFF5470),
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
            Modifier.fillMaxSize().background(Brush.verticalGradient(listOf(Color(0xFF1C0B38), Graphite, Color(0xFF0D0620))))
                .systemBarsPadding()
        ) {
            Column(Modifier.fillMaxSize()) {
                SunsetHero(onName = { nameEdit = true })
            Column(Modifier.fillMaxSize().padding(horizontal = if (wide) 40.dp else 16.dp, vertical = 16.dp)) {
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
        }
        if (manual) ManualDialog(onDismiss = { manual = false }) { h, p -> manual = false; connect(h, p, null, null) }
        if (nameEdit) NameDialog { nameEdit = false }
        pinFor.value?.let { p -> PinDialog(onCancel = { NativeCore.close(p.handle); pinFor.value = null }) { submitPin(p, it) } }
    }

    @Composable
    private fun SunsetHero(onName: () -> Unit) {
        Box(Modifier.fillMaxWidth().height(190.dp)) {
            SunsetScene(Modifier.matchParentSize())
            TextButton(onClick = onName, modifier = Modifier.align(Alignment.TopEnd).padding(8.dp)) {
                Icon(Icons.Filled.Tablet, null, tint = TextHi)
                Spacer(Modifier.width(6.dp))
                Text(storage.deviceName, color = TextHi, maxLines = 1)
            }
            Column(Modifier.align(Alignment.BottomCenter).padding(bottom = 44.dp), horizontalAlignment = Alignment.CenterHorizontally) {
                PosterTitle("KB STUDIO SCREEN", 44.sp)
                Text(stringResource(R.string.brand).uppercase(), color = Cyan, fontFamily = Display, fontStyle = FontStyle.Italic, fontSize = 18.sp,
                    style = LocalTextStyle.current.copy(shadow = Shadow(Color(0xFF140726), Offset(3f, 3f), 0f)))
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

/** Título de afiche: blanco con contorno oscuro y sombra rosa desplazada. */
@Composable
fun PosterTitle(text: String, size: androidx.compose.ui.unit.TextUnit) {
    Box {
        Text(text, color = Color(0xFF140726), fontFamily = Display, fontStyle = FontStyle.Italic, fontSize = size, modifier = Modifier.offset(5.dp, 5.dp))
        Text(text, color = Orange, fontFamily = Display, fontStyle = FontStyle.Italic, fontSize = size, modifier = Modifier.offset(2.5.dp, 2.5.dp))
        Text(text, color = Color.White, fontFamily = Display, fontStyle = FontStyle.Italic, fontSize = size)
    }
}

/** Escena de atardecer: cielo degradado, sol con franjas, grilla neón y palmeras (dibujo propio). */
@Composable
fun SunsetScene(modifier: Modifier) {
    Canvas(modifier) {
        val w = size.width; val h = size.height; val horizon = h * 0.80f
        drawRect(Brush.verticalGradient(listOf(Color(0xFF1A0838), Color(0xFF5B1A73), Color(0xFFC42D7A), Color(0xFFFF6B4A), Color(0xFFFFB35C)), endY = horizon), size = androidx.compose.ui.geometry.Size(w, horizon))
        // Sol con franjas horizontales.
        val r = h * 0.42f; val cx = w / 2; val cy = horizon - r * 0.15f
        val sun = Brush.verticalGradient(listOf(Color(0xFFFFF1A8), Color(0xFFFFB347), Color(0xFFFF3D7F)), startY = cy - r, endY = cy + r)
        clipRect(bottom = horizon) {
            drawCircle(sun, r, Offset(cx, cy))
            var y = cy + r * 0.05f; var gap = 3f
            while (y < cy + r) { drawRect(Color(0xFFC42D7A).copy(alpha = 0.95f), Offset(cx - r, y), androidx.compose.ui.geometry.Size(2 * r, gap)); y += 14f; gap += 1.6f }
        }
        // Suelo con grilla en perspectiva.
        drawRect(Brush.verticalGradient(listOf(Color(0xFF2A0F4A), Color(0xFF140726)), startY = horizon, endY = h), Offset(0f, horizon), androidx.compose.ui.geometry.Size(w, h - horizon))
        val gridColor = Cyan.copy(alpha = 0.85f)
        for (i in -12..12) drawLine(gridColor, Offset(cx + i * w * 0.02f, horizon), Offset(cx + i * w * 0.16f, h), 1.5f)
        var t = 0.08f
        while (t < 1f) { val yy = horizon + (h - horizon) * t * t; drawLine(gridColor, Offset(0f, yy), Offset(w, yy), 1.5f); t += 0.16f }
        drawLine(Brush.horizontalGradient(listOf(Color(0xFFFF2E88), Color(0xFFFFD166))), Offset(0f, h - 2f), Offset(w, h - 2f), 4f)
        // Palmeras.
        palm(w * 0.07f, h, h * 0.95f, 1f); palm(w * 0.20f, h, h * 0.62f, -1f)
        palm(w * 0.93f, h, h * 0.92f, -1f); palm(w * 0.81f, h, h * 0.58f, 1f)
    }
}

private fun androidx.compose.ui.graphics.drawscope.DrawScope.palm(x: Float, base: Float, height: Float, lean: Float) {
    val c = Color(0xFF140726)
    val top = Offset(x + lean * height * 0.12f, base - height)
    val trunk = androidx.compose.ui.graphics.Path().apply {
        moveTo(x - height * 0.025f, base); quadraticTo(x + lean * height * 0.02f, base - height * 0.5f, top.x - height * 0.012f, top.y)
        lineTo(top.x + height * 0.012f, top.y); quadraticTo(x + lean * height * 0.05f, base - height * 0.5f, x + height * 0.025f, base); close()
    }
    drawPath(trunk, c)
    val fronds = listOf(-160f, -125f, -95f, -60f, -25f, 10f)
    for (a in fronds) {
        val rad = Math.toRadians(a.toDouble()); val len = height * 0.42f
        val end = Offset(top.x + (kotlin.math.cos(rad) * len).toFloat(), top.y + (kotlin.math.sin(rad) * len * 0.7f).toFloat() + len * 0.25f)
        val mid = Offset((top.x + end.x) / 2, minOf(top.y, end.y) - len * 0.18f)
        val leaf = androidx.compose.ui.graphics.Path().apply {
            moveTo(top.x, top.y); quadraticTo(mid.x, mid.y - len * 0.06f, end.x, end.y); quadraticTo(mid.x, mid.y + len * 0.10f, top.x, top.y + len * 0.04f); close()
        }
        drawPath(leaf, c)
    }
}

@Composable
fun Glass(modifier: Modifier = Modifier, content: @Composable () -> Unit) {
    // Vidrio violeta con borde neón rosado, sin perder contraste.
    Surface(
        modifier = modifier.fillMaxWidth(), shape = RoundedCornerShape(18.dp),
        color = Panel.copy(alpha = 0.85f), border = BorderStroke(1.dp, Orange.copy(alpha = 0.28f)),
    ) { content() }
}

@Composable
fun SectionTitle(t: String) = Text(t.uppercase(), color = Yellow, fontFamily = Display, fontStyle = FontStyle.Italic, fontSize = 22.sp,
    style = LocalTextStyle.current.copy(shadow = Shadow(Color(0xFF140726), Offset(3f, 3f), 0f)))

@Composable
fun BigButton(label: String, icon: androidx.compose.ui.graphics.vector.ImageVector, modifier: Modifier, outlined: Boolean = false, onClick: () -> Unit) {
    val shape = RoundedCornerShape(16.dp)
    val base = modifier.heightIn(min = 60.dp).clip(shape)
    val styled = if (outlined) base.border(BorderStroke(1.5.dp, Cyan), shape).background(Color(0xFF140726).copy(alpha = 0.6f)) else base.background(Sunset)
    Row(styled.clickable(onClick = onClick).padding(horizontal = 16.dp), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.Center) {
        val fg = if (outlined) Cyan else Color(0xFF1A0716)
        Icon(icon, null, tint = fg); Spacer(Modifier.width(10.dp))
        Text(label.uppercase(), color = fg, fontFamily = Display, fontStyle = FontStyle.Italic, fontSize = 20.sp)
    }
}

@Composable
fun HostRow(name: String, addr: String, badge: String, paired: Boolean, onForget: (() -> Unit)? = null, onClick: () -> Unit) {
    Glass(Modifier.clickable(onClick = onClick)) {
        Row(Modifier.padding(horizontal = 18.dp, vertical = 16.dp).heightIn(min = 40.dp), verticalAlignment = Alignment.CenterVertically) {
            Icon(Icons.Filled.Computer, null, tint = if (paired) Orange else Cyan, modifier = Modifier.size(30.dp))
            Spacer(Modifier.width(14.dp))
            Column(Modifier.weight(1f)) {
                Text(name.uppercase(), color = TextHi, fontSize = 22.sp, fontFamily = Display, fontStyle = FontStyle.Italic)
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
