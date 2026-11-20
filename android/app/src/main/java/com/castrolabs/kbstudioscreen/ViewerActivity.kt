package com.castrolabs.kbstudioscreen

import android.annotation.SuppressLint
import android.content.Context
import android.graphics.Matrix
import android.graphics.SurfaceTexture
import android.net.wifi.WifiManager
import android.os.Bundle
import android.view.Gravity
import android.view.MotionEvent
import android.view.Surface
import android.view.TextureView
import android.view.ViewGroup
import android.view.WindowManager
import android.widget.FrameLayout
import android.widget.Toast
import androidx.activity.ComponentActivity
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.ComposeView
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.core.view.WindowCompat
import androidx.core.view.WindowInsetsCompat
import androidx.core.view.WindowInsetsControllerCompat
import kotlinx.coroutines.delay
import org.json.JSONArray
import org.json.JSONObject
import kotlin.math.abs
import kotlin.math.hypot
import kotlin.math.sign

/**
 * Pantalla de visualización/control a pantalla completa.
 *
 * Gestos:
 *  - 1 dedo: arrastre sostenido enviado a Windows (fader, clic, arrastrar ventana).
 *  - 2 dedos pellizcando: zoom LOCAL del visor (nunca llega a Windows).
 *  - 2 dedos desplazando: rueda del mouse en Windows (o paneo si hay zoom).
 *  - "Tacto bloqueado": nada llega a Windows; sólo navegación local.
 */
class ViewerActivity : ComponentActivity(), TextureView.SurfaceTextureListener {
    private lateinit var texture: TextureView
    private var decoder: H264Decoder? = null
    private var surface: Surface? = null
    @Volatile private var handle = 0L
    @Volatile private var alive = true
    private var wifiLock: WifiManager.WifiLock? = null

    // Estado de presentación
    private var frameW = 0
    private var frameH = 0
    private var zoom = 1f
    private var offX = 0f
    private var offY = 0f

    // Estado observable por la UI
    private val permission = mutableStateOf("view")
    private val locked = mutableStateOf(false)
    private val reconnecting = mutableStateOf(false)
    private val fps = mutableIntStateOf(0)
    private val toolbar = mutableStateOf(true)
    private val monitors = mutableStateOf<List<Pair<String, String>>>(emptyList())
    private val currentMonitor = mutableStateOf("")

    @SuppressLint("ClickableViewAccessibility")
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        handle = SessionHolder.handle
        if (handle == 0L) { finish(); return }
        window.addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
        WindowCompat.setDecorFitsSystemWindows(window, false)
        WindowInsetsControllerCompat(window, window.decorView).apply {
            hide(WindowInsetsCompat.Type.systemBars())
            systemBarsBehavior = WindowInsetsControllerCompat.BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE
        }
        // Wi-Fi en modo de baja latencia mientras se transmite (Android 10+).
        val wm = applicationContext.getSystemService(Context.WIFI_SERVICE) as WifiManager
        wifiLock = wm.createWifiLock(WifiManager.WIFI_MODE_FULL_LOW_LATENCY, "kbss-stream").apply { setReferenceCounted(false); acquire() }

        texture = TextureView(this).apply { surfaceTextureListener = this@ViewerActivity }
        texture.setOnTouchListener { _, ev -> onTouch(ev); true }
        val overlay = ComposeView(this).apply { setContent { MaterialTheme { Overlay() } } }
        setContentView(FrameLayout(this).apply {
            setBackgroundColor(android.graphics.Color.BLACK)
            addView(texture, FrameLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.MATCH_PARENT))
            addView(overlay, FrameLayout.LayoutParams(ViewGroup.LayoutParams.WRAP_CONTENT, ViewGroup.LayoutParams.WRAP_CONTENT, Gravity.TOP or Gravity.CENTER_HORIZONTAL))
        })
        readStage()
        texture.addOnLayoutChangeListener { _, _, _, _, _, _, _, _, _ -> applyTransform() }
    }

    private fun readStage() {
        runCatching {
            val st = JSONObject(NativeCore.stage(handle))
            permission.value = st.optString("permission", "view")
            currentMonitor.value = st.optString("monitor", "")
        }
        refreshMonitors()
    }

    private fun refreshMonitors() {
        runCatching {
            val a = JSONArray(NativeCore.monitors(handle))
            monitors.value = (0 until a.length()).map {
                val o = a.getJSONObject(it)
                o.getString("id") to "${o.getString("name")} (${o.getInt("w")}×${o.getInt("h")})"
            }
        }
    }

    // ------------------------------------------------------------ sesión

    override fun onSurfaceTextureAvailable(st: SurfaceTexture, w: Int, h: Int) {
        val s = Surface(st)
        surface = s
        startStreaming(s)
    }

    private fun startStreaming(s: Surface) {
        val h = handle
        val dec = H264Decoder(s, { fw, fh -> runOnUiThread { frameW = fw; frameH = fh; applyTransform() } }) { NativeCore.requestKeyframe(h) }
        decoder = dec
        Thread({
            while (alive) {
                val f = NativeCore.nextFrame(h) ?: break
                dec.feed(f)
            }
        }, "kbss-frames").start()
        Thread({
            while (alive) {
                val ev = JSONObject(NativeCore.nextControl(h))
                when (ev.optString("type")) {
                    "session" -> runOnUiThread {
                        permission.value = ev.optString("permission", "view")
                        currentMonitor.value = ev.optString("monitor", "")
                    }
                    "monitors" -> runOnUiThread { refreshMonitors() }
                    "mode_result" -> if (!ev.optBoolean("accepted")) runOnUiThread {
                        Toast.makeText(this, ev.optString("reason"), Toast.LENGTH_SHORT).show()
                    }
                    "closed" -> { if (alive) reconnect(); break }
                    "error" -> if (ev.optString("code") == "NotPaired") runOnUiThread {
                        Toast.makeText(this, R.string.err_not_paired, Toast.LENGTH_LONG).show(); finish()
                    }
                }
            }
        }, "kbss-control").start()
        Thread({
            while (alive) {
                Thread.sleep(1000)
                val n = dec.rendered.getAndSet(0)
                runOnUiThread { fps.intValue = n }
            }
        }, "kbss-stats").start()
    }

    /** Reintenta con la huella guardada cada 2 s mientras la pantalla siga abierta. */
    private fun reconnect() {
        runOnUiThread { reconnecting.value = true }
        val host = SessionHolder.host ?: return
        val storage = Storage(this)
        val (cert, key) = storage.identity()
        val info = DeviceInfo.detect(this)
        val old = handle
        handle = 0
        decoder?.release()
        NativeCore.close(old)
        while (alive) {
            try {
                val h = NativeCore.connect(host.host, host.port, NativeCore.MODE_TRUSTED, host.fp, "", cert, key, storage.deviceName,
                    info.isTablet, info.widthPx, info.heightPx, info.dpi, info.refresh, info.decMaxW, info.decMaxH)
                if (JSONObject(NativeCore.stage(h)).getString("stage") == "need_pin") {
                    NativeCore.close(h)
                    runOnUiThread { Toast.makeText(this, R.string.err_not_paired, Toast.LENGTH_LONG).show(); finish() }
                    return
                }
                handle = h
                SessionHolder.handle = h
                runOnUiThread {
                    reconnecting.value = false
                    readStage()
                    applyTransform()
                    surface?.let { startStreaming(it) }
                }
                return
            } catch (_: Throwable) {
                Thread.sleep(2000)
            }
        }
    }

    override fun onDestroy() {
        alive = false
        decoder?.release()
        if (handle != 0L) NativeCore.close(handle)
        SessionHolder.handle = 0
        wifiLock?.let { if (it.isHeld) it.release() }
        super.onDestroy()
    }

    override fun onSurfaceTextureSizeChanged(st: SurfaceTexture, w: Int, h: Int) = applyTransform()
    override fun onSurfaceTextureDestroyed(st: SurfaceTexture): Boolean { decoder?.release(); return true }
    override fun onSurfaceTextureUpdated(st: SurfaceTexture) {}

    // ------------------------------------------------------------ geometría

    /** Letterbox + zoom local. Debe coincidir con kb-geometry (ClientViewport). */
    private fun applyTransform() {
        val vw = texture.width.toFloat()
        val vh = texture.height.toFloat()
        if (vw <= 0 || vh <= 0 || frameW == 0 || frameH == 0) return
        val s = minOf(vw / frameW, vh / frameH)
        val cw = frameW * s
        val ch = frameH * s
        val m = Matrix()
        m.setScale(cw / vw, ch / vh)
        m.postTranslate((vw - cw) / 2, (vh - ch) / 2)
        m.postScale(zoom, zoom)
        m.postTranslate(offX, offY)
        texture.setTransform(m)
        if (handle != 0L) NativeCore.setViewport(handle, vw, vh, frameW, frameH, zoom, offX, offY)
    }

    private fun clampPan() {
        val vw = texture.width.toFloat()
        val vh = texture.height.toFloat()
        offX = offX.coerceIn(vw - vw * zoom, 0f)
        offY = offY.coerceIn(vh - vh * zoom, 0f)
    }

    // ------------------------------------------------------------ gestos

    private var sentDown = false
    private var twoFinger = false
    private var lastDist = 0f
    private var lastCx = 0f
    private var lastCy = 0f
    private var wheelAcc = 0f
    private var panLastX = 0f
    private var panLastY = 0f

    private fun onTouch(ev: MotionEvent) {
        val h = handle
        if (h == 0L) return
        val canSend = permission.value == "control" && !locked.value
        when (ev.actionMasked) {
            MotionEvent.ACTION_DOWN -> {
                twoFinger = false
                sentDown = canSend && NativeCore.touch(h, NativeCore.PHASE_DOWN, ev.x, ev.y)
                panLastX = ev.x; panLastY = ev.y
            }
            MotionEvent.ACTION_POINTER_DOWN -> if (ev.pointerCount == 2) {
                if (sentDown) { NativeCore.touch(h, NativeCore.PHASE_CANCEL, ev.getX(0), ev.getY(0)); sentDown = false }
                twoFinger = true
                lastDist = dist(ev); lastCx = cx(ev); lastCy = cy(ev); wheelAcc = 0f
            }
            MotionEvent.ACTION_MOVE -> when {
                twoFinger && ev.pointerCount >= 2 -> {
                    val d = dist(ev); val x = cx(ev); val y = cy(ev)
                    if (lastDist > 0 && abs(d / lastDist - 1f) > 0.008f) {
                        val f = d / lastDist
                        val ns = (zoom * f).coerceIn(1f, 6f)
                        val k = ns / zoom
                        offX = x - (x - offX) * k; offY = y - (y - offY) * k; zoom = ns
                    }
                    val dy = y - lastCy
                    if (zoom > 1.01f) {
                        offX += x - lastCx; offY += dy
                    } else if (canSend) {
                        wheelAcc += dy
                        val step = 40f * resources.displayMetrics.density
                        while (abs(wheelAcc) >= step) {
                            NativeCore.wheel(h, x, y, (120 * sign(wheelAcc)).toInt())
                            wheelAcc -= step * sign(wheelAcc)
                        }
                    }
                    clampPan(); applyTransform()
                    lastDist = d; lastCx = x; lastCy = y
                }
                sentDown -> NativeCore.touch(h, NativeCore.PHASE_MOVE, ev.x, ev.y)
                !twoFinger && zoom > 1.01f -> {
                    // Sin control (solo ver o bloqueado): un dedo panea el zoom.
                    offX += ev.x - panLastX; offY += ev.y - panLastY
                    panLastX = ev.x; panLastY = ev.y
                    clampPan(); applyTransform()
                }
            }
            MotionEvent.ACTION_UP -> {
                if (sentDown) NativeCore.touch(h, NativeCore.PHASE_UP, ev.x, ev.y)
                sentDown = false; twoFinger = false
            }
            MotionEvent.ACTION_CANCEL -> {
                if (sentDown) NativeCore.touch(h, NativeCore.PHASE_CANCEL, ev.x, ev.y)
                sentDown = false; twoFinger = false
            }
        }
    }

    private fun dist(e: MotionEvent) = hypot(e.getX(0) - e.getX(1), e.getY(0) - e.getY(1))
    private fun cx(e: MotionEvent) = (e.getX(0) + e.getX(1)) / 2
    private fun cy(e: MotionEvent) = (e.getY(0) + e.getY(1)) / 2

    // ------------------------------------------------------------ overlay

    @Composable
    private fun Overlay() {
        LaunchedEffect(toolbar.value) { if (toolbar.value) { delay(5000); toolbar.value = false } }
        Column(horizontalAlignment = Alignment.CenterHorizontally, modifier = Modifier.padding(top = 8.dp)) {
            if (reconnecting.value) Pill(stringResource(R.string.reconnecting), Orange)
            if (toolbar.value) {
                Surface(shape = RoundedCornerShape(16.dp), color = Panel.copy(alpha = 0.88f), border = BorderStroke(1.dp, Color.White.copy(alpha = 0.1f))) {
                    Row(Modifier.padding(horizontal = 8.dp, vertical = 6.dp), verticalAlignment = Alignment.CenterVertically) {
                        ToolButton(Icons.Filled.Close, stringResource(R.string.disconnect)) { finish() }
                        val ctrl = permission.value == "control"
                        Pill(stringResource(if (ctrl) R.string.control else R.string.view_only), if (ctrl) Orange else Metal)
                        if (ctrl) {
                            MonitorMenu()
                            ToolButton(if (locked.value) Icons.Filled.TouchApp else Icons.Filled.DoNotTouch,
                                stringResource(if (locked.value) R.string.touch_locked else R.string.touch_unlocked),
                                tint = if (locked.value) Orange else TextHi) { locked.value = !locked.value }
                        }
                        ToolButton(Icons.Filled.ZoomOutMap, stringResource(R.string.reset_zoom)) { zoom = 1f; offX = 0f; offY = 0f; applyTransform() }
                        ToolButton(Icons.Filled.Refresh, stringResource(R.string.refresh_image)) { if (handle != 0L) NativeCore.requestKeyframe(handle) }
                        Text("${frameW}×${frameH} · ${fps.intValue} fps", color = Metal, fontSize = 13.sp, fontFamily = FontFamily.Monospace, modifier = Modifier.padding(horizontal = 10.dp))
                    }
                }
            } else {
                // Manija discreta para volver a mostrar la barra.
                Box(Modifier.size(width = 72.dp, height = 22.dp).clickable { toolbar.value = true }, contentAlignment = Alignment.Center) {
                    Surface(shape = RoundedCornerShape(50), color = Color.White.copy(alpha = 0.22f)) { Spacer(Modifier.size(width = 44.dp, height = 5.dp)) }
                }
            }
        }
    }

    @Composable
    private fun MonitorMenu() {
        var open by remember { mutableStateOf(false) }
        Box {
            ToolButton(Icons.Filled.Monitor, stringResource(R.string.source_monitor)) { refreshMonitors(); open = true }
            DropdownMenu(expanded = open, onDismissRequest = { open = false }) {
                monitors.value.forEach { (id, label) ->
                    DropdownMenuItem(
                        text = { Text(label + if (id == currentMonitor.value) "  ✓" else "") },
                        onClick = { open = false; if (handle != 0L) NativeCore.selectMonitor(handle, id) })
                }
            }
        }
    }
}

@Composable
fun ToolButton(icon: ImageVector, desc: String, tint: Color = TextHi, onClick: () -> Unit) {
    // 48 dp mínimo: tamaño táctil accesible también en teléfonos.
    IconButton(onClick, Modifier.size(52.dp)) { Icon(icon, desc, tint = tint) }
}

@Composable
fun Pill(text: String, color: Color) {
    Surface(shape = RoundedCornerShape(50), color = color.copy(alpha = 0.18f), border = BorderStroke(1.dp, color.copy(alpha = 0.6f)), modifier = Modifier.padding(4.dp)) {
        Text(text, color = color, fontSize = 13.sp, modifier = Modifier.padding(horizontal = 12.dp, vertical = 6.dp))
    }
}
