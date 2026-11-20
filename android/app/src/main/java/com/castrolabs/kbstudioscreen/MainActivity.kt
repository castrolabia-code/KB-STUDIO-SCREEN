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
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.res.imageResource
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

// Paleta "Studio": racks negros, celeste de estudio, displays LCD y LEDs de estado.
val Graphite = Color(0xFF0D1015)   // fondo
val Panel = Color(0xFF171C23)      // módulo
val Panel2 = Color(0xFF1D232C)
val Edge = Color(0xFF2A323D)
val Metal = Color(0xFF8B96A5)      // texto secundario
val TextHi = Color(0xFFE8EEF4)
val Orange = Color(0xFF3D8BFF)     // acento principal: azul eléctrico de los LEDs del estudio
val StudioBlue2 = Color(0xFF8CC2FF)
val Amber = Color(0xFFF2A54A)      // luz cálida / madera
val Lcd = Color(0xFF081425)
val LcdText = Color(0xFF9CC9FF)
val LedOn = Color(0xFF47D985)
val LedWait = Color(0xFFF5B841)
val Yellow = LedWait
val Cyan = StudioBlue2
/** Inter (SIL OFL 1.1): tipografía limpia y técnica. */
val Display = FontFamily(Font(R.font.inter_regular, FontWeight.Normal), Font(R.font.inter_semibold, FontWeight.SemiBold), Font(R.font.inter_bold, FontWeight.Bold))

private val KbColors = darkColorScheme(
    primary = Orange, onPrimary = Color(0xFF04202F), background = Graphite, surface = Panel,
    onSurface = TextHi, onBackground = TextHi, secondary = StudioBlue2, tertiary = LedWait, error = Color(0xFFFF5A5F),
)

class MainActivity : ComponentActivity() {
    private lateinit var storage: Storage
    private var discovery: Discovery? = null

    private val found = mutableStateOf<List<FoundHost>>(emptyList())
    private val saved = mutableStateOf<List<SavedHost>>(emptyList())
    private val busy = mutableStateOf<String?>(null)
    private val error = mutableStateOf<String?>(null)
    private val pinFor = mutableStateOf<PendingPin?>(null)
    // Foto de ambientación elegida por el usuario: queda sólo en este dispositivo.
    private val bgFile by lazy { java.io.File(filesDir, "background.img") }
    private val bgImage = mutableStateOf<androidx.compose.ui.graphics.ImageBitmap?>(null)

    private val pickBg = registerForActivityResult(androidx.activity.result.contract.ActivityResultContracts.PickVisualMedia()) { uri ->
        if (uri != null) runCatching {
            contentResolver.openInputStream(uri)?.use { inp -> bgFile.outputStream().use { inp.copyTo(it) } }
            loadBg()
        }
    }

    private fun loadBg() {
        bgImage.value = runCatching {
            if (!bgFile.exists()) return@runCatching null
            // Decodificar reducida (ahorra memoria en tablets de 4 GB).
            val o = android.graphics.BitmapFactory.Options().apply { inJustDecodeBounds = true }
            android.graphics.BitmapFactory.decodeFile(bgFile.path, o)
            var sample = 1
            while (o.outWidth / sample > 1600) sample *= 2
            android.graphics.BitmapFactory.decodeFile(bgFile.path, android.graphics.BitmapFactory.Options().apply { inSampleSize = sample })?.asImageBitmap()
        }.getOrNull()
    }

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
        loadBg()
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
            Modifier.fillMaxSize().background(Brush.verticalGradient(listOf(Color(0xFF131922), Graphite)))
                .systemBarsPadding()
        ) {
            Column(Modifier.fillMaxSize()) {
                SunsetHero(onName = { nameEdit = true })
                // Foto propia si el usuario eligió una; si no, la ilustración de estudio incluida.
                (bgImage.value ?: androidx.compose.ui.graphics.ImageBitmap.imageResource(R.drawable.estudio_kbss)).let { img ->
                    Box(Modifier.fillMaxWidth().padding(horizontal = 12.dp).height(if (wide) 240.dp else 170.dp).clip(RoundedCornerShape(bottomStart = 12.dp, bottomEnd = 12.dp))) {
                        androidx.compose.foundation.Image(img, null, contentScale = androidx.compose.ui.layout.ContentScale.Crop, modifier = Modifier.matchParentSize())
                        Box(Modifier.matchParentSize().background(Brush.verticalGradient(listOf(Color.Transparent, Color(0x330D1015), Color(0xCC0D1015)))))
                    }
                }
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

    /** Cabecera: frente de rack con tornillos, logo de barras de nivel, título y dispositivo. */
    @Composable
    private fun SunsetHero(onName: () -> Unit) {
        Surface(
            modifier = Modifier.fillMaxWidth().padding(12.dp), shape = RoundedCornerShape(12.dp),
            color = Color(0xFF1C222B), border = BorderStroke(1.dp, Color(0xFF38424F)),
        ) {
            val narrow = LocalConfiguration.current.screenWidthDp < 600
            Column(Modifier.padding(horizontal = 14.dp, vertical = 12.dp)) {
                // Fila 1: tornillo · logo · nombre · tornillo (el nombre nunca se parte).
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Screw(); Spacer(Modifier.width(12.dp))
                    LevelLogo(); Spacer(Modifier.width(12.dp))
                    Column(Modifier.weight(1f)) {
                        // Nombre: blanco, grueso y con sombreado profundo para máximo contraste.
                        Text("KB STUDIO SCREEN", color = Color.White, fontFamily = Display, fontWeight = FontWeight.Bold,
                            fontSize = if (narrow) 19.sp else 24.sp, letterSpacing = if (narrow) 1.sp else 2.sp, maxLines = 1, softWrap = false,
                            overflow = androidx.compose.ui.text.style.TextOverflow.Ellipsis,
                            style = LocalTextStyle.current.copy(shadow = Shadow(Color.Black, Offset(0f, 5f), 10f)))
                        Text(stringResource(R.string.brand).uppercase(), color = Amber, fontFamily = Display, fontWeight = FontWeight.Bold, fontSize = 11.sp,
                            letterSpacing = 3.sp, maxLines = 1, softWrap = false,
                            style = LocalTextStyle.current.copy(shadow = Shadow(Color.Black, Offset(0f, 3f), 6f)))
                    }
                    Spacer(Modifier.width(8.dp)); Screw()
                }
                Spacer(Modifier.height(10.dp))
                Box(Modifier.fillMaxWidth().height(1.dp).background(Color(0xFF2A323D)))
                Spacer(Modifier.height(4.dp))
                // Fila 2: este dispositivo (tocar para renombrar) · foto de fondo.
                Row(verticalAlignment = Alignment.CenterVertically) {
                    TextButton(onClick = onName, modifier = Modifier.weight(1f)) {
                        Icon(Icons.Filled.Tablet, null, tint = Metal, modifier = Modifier.size(18.dp))
                        Spacer(Modifier.width(8.dp))
                        Text(storage.deviceName, color = Metal, maxLines = 1, fontSize = 13.sp, overflow = androidx.compose.ui.text.style.TextOverflow.Ellipsis,
                            modifier = Modifier.weight(1f))
                    }
                    var bgMenu by remember { mutableStateOf(false) }
                    Box {
                        IconButton({ bgMenu = true }, Modifier.size(48.dp)) { Icon(Icons.Filled.Image, stringResource(R.string.bg_photo), tint = Metal) }
                        DropdownMenu(bgMenu, { bgMenu = false }) {
                            DropdownMenuItem(text = { Text(stringResource(R.string.bg_pick)) }, onClick = {
                                bgMenu = false
                                pickBg.launch(androidx.activity.result.PickVisualMediaRequest(androidx.activity.result.contract.ActivityResultContracts.PickVisualMedia.ImageOnly))
                            })
                            if (bgImage.value != null) DropdownMenuItem(text = { Text(stringResource(R.string.bg_remove)) }, onClick = {
                                bgMenu = false; bgFile.delete(); loadBg()
                            })
                        }
                    }
                }
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

/** Tornillo de rack. */
@Composable
fun Screw() {
    Box(Modifier.size(12.dp).clip(RoundedCornerShape(50)).background(Brush.radialGradient(listOf(Color(0xFF9AA5B3), Color(0xFF4B5563), Color(0xFF2B313A)))))
}

/** Logo: barras de nivel (vúmetro) en celeste. */
@Composable
fun LevelLogo() {
    Canvas(Modifier.size(34.dp).clip(RoundedCornerShape(8.dp)).background(Color(0xFF11161D)).border(1.dp, Color(0xFF38424F), RoundedCornerShape(8.dp))) {
        val bars = listOf(0.30f, 0.48f, 0.60f, 0.42f)
        val bw = size.width * 0.09f
        bars.forEachIndexed { i, h ->
            val x = size.width * (0.22f + i * 0.155f)
            val top = size.height * (0.80f - h)
            drawRoundRect(if (i == 2) Amber else Orange, Offset(x, top), androidx.compose.ui.geometry.Size(bw, size.height * 0.80f - top),
                androidx.compose.ui.geometry.CornerRadius(2f, 2f))
        }
    }
}

/** Display tipo LCD para datos técnicos. */
@Composable
fun LcdText(text: String, modifier: Modifier = Modifier) {
    Text(text, color = LcdText, fontFamily = FontFamily.Monospace, fontSize = 13.sp,
        modifier = modifier.clip(RoundedCornerShape(6.dp)).background(Lcd).border(1.dp, Color(0xFF12303D), RoundedCornerShape(6.dp)).padding(horizontal = 10.dp, vertical = 6.dp))
}

@Composable
fun Glass(modifier: Modifier = Modifier, content: @Composable () -> Unit) {
    // Módulo de rack: superficie oscura con borde sutil.
    Surface(
        modifier = modifier.fillMaxWidth(), shape = RoundedCornerShape(10.dp),
        color = Panel, border = BorderStroke(1.dp, Edge),
    ) { content() }
}

@Composable
fun SectionTitle(t: String) {
    Row(verticalAlignment = Alignment.CenterVertically) {
        Box(Modifier.size(width = 3.dp, height = 14.dp).clip(RoundedCornerShape(2.dp)).background(Orange))
        Spacer(Modifier.width(10.dp))
        Text(t.uppercase(), color = Metal, fontFamily = Display, fontWeight = FontWeight.SemiBold, fontSize = 12.sp, letterSpacing = 2.sp)
    }
}

@Composable
fun BigButton(label: String, icon: androidx.compose.ui.graphics.vector.ImageVector, modifier: Modifier, outlined: Boolean = false, onClick: () -> Unit) {
    val shape = RoundedCornerShape(10.dp)
    val base = modifier.heightIn(min = 56.dp).clip(shape)
    val styled = if (outlined) base.border(1.dp, Color(0xFF38424F), shape).background(Brush.verticalGradient(listOf(Color(0xFF262E38), Color(0xFF1E252E))))
    else base.background(Brush.verticalGradient(listOf(Color(0xFF5A9DFF), Color(0xFF2F74E6))))
    Row(styled.clickable(onClick = onClick).padding(horizontal = 16.dp), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.Center) {
        val fg = if (outlined) TextHi else Color.White
        val narrow = LocalConfiguration.current.screenWidthDp < 600
        Icon(icon, null, tint = fg, modifier = Modifier.size(if (narrow) 20.dp else 24.dp)); Spacer(Modifier.width(if (narrow) 8.dp else 10.dp))
        Text(label, color = fg, fontFamily = Display, fontWeight = FontWeight.SemiBold, fontSize = if (narrow) 14.sp else 16.sp, maxLines = 1,
            overflow = androidx.compose.ui.text.style.TextOverflow.Ellipsis)
    }
}

@Composable
fun HostRow(name: String, addr: String, badge: String, paired: Boolean, onForget: (() -> Unit)? = null, onClick: () -> Unit) {
    Glass(Modifier.clickable(onClick = onClick)) {
        Row(Modifier.padding(horizontal = 18.dp, vertical = 16.dp).heightIn(min = 40.dp), verticalAlignment = Alignment.CenterVertically) {
            Box(Modifier.size(10.dp).clip(RoundedCornerShape(50)).background(if (paired) LedOn else LedWait))
            Spacer(Modifier.width(12.dp))
            Icon(Icons.Filled.Computer, null, tint = Orange, modifier = Modifier.size(26.dp))
            Spacer(Modifier.width(12.dp))
            Column(Modifier.weight(1f)) {
                Text(name, color = TextHi, fontSize = 17.sp, fontFamily = Display, fontWeight = FontWeight.SemiBold)
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
