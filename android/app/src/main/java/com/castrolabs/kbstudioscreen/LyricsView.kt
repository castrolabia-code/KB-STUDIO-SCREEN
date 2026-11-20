package com.castrolabs.kbstudioscreen

import android.graphics.Bitmap
import android.graphics.Color as AColor
import android.graphics.pdf.PdfRenderer
import android.os.ParcelFileDescriptor
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.gestures.animateScrollBy
import androidx.compose.foundation.gestures.scrollBy
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.ColorFilter
import androidx.compose.ui.graphics.ColorMatrix
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext
import java.io.File
import java.nio.charset.Charset
import java.nio.charset.CodingErrorAction
import java.nio.ByteBuffer

/** Documento recibido de la PC. Se guarda en la caché privada y se borra al cerrar. */
data class LyricsDoc(val file: File, val isPdf: Boolean, val name: String)

/** Estado del teleprompter: lo cambian el cantante (si no está bloqueado) o la PC. */
class PrompterState {
    var speed by mutableIntStateOf(0)        // 0 = detenido, 1..10
    var locked by mutableStateOf(false)      // bloqueado por la PC
    var fontSp by mutableFloatStateOf(44f)
    var theme by mutableIntStateOf(0)        // 0 blanco/negro, 1 negro/blanco, 2 amarillo/negro
    /** +1 siguiente, -1 anterior, 1000+N ir a la página N. */
    val pageCommands = MutableSharedFlow<Int>(extraBufferCapacity = 8)
}

private data class LyricsTheme(val bg: Color, val fg: Color, val invertPdf: Boolean)
private val Themes = listOf(
    LyricsTheme(Color.Black, Color.White, true),
    LyricsTheme(Color.White, Color.Black, false),
    LyricsTheme(Color.Black, Color(0xFFFFD54F), true),
)

/** Texto: UTF-8 si es válido; si no, Windows-1252 (habitual en .txt de Windows). */
fun decodeText(bytes: ByteArray): String = try {
    Charsets.UTF_8.newDecoder().onMalformedInput(CodingErrorAction.REPORT).decode(ByteBuffer.wrap(bytes)).toString()
} catch (_: Exception) {
    String(bytes, Charset.forName("windows-1252"))
}.removePrefix("﻿")

@Composable
fun LyricsScreen(doc: LyricsDoc, st: PrompterState, onClose: () -> Unit) {
    val theme = Themes[st.theme]
    Box(Modifier.fillMaxSize().background(theme.bg)) {
        if (doc.isPdf) PdfPages(doc.file, theme, st) else TextLyrics(doc.file, theme, st)

        if (st.locked) {
            Icon(Icons.Filled.Lock, null, tint = theme.fg.copy(alpha = 0.35f), modifier = Modifier.align(Alignment.TopEnd).padding(12.dp).size(28.dp))
        } else {
            Surface(
                modifier = Modifier.align(Alignment.BottomCenter).padding(12.dp),
                shape = RoundedCornerShape(18.dp), color = Panel.copy(alpha = 0.9f),
            ) {
                Row(Modifier.padding(horizontal = 6.dp, vertical = 4.dp), verticalAlignment = Alignment.CenterVertically) {
                    ToolButton(Icons.Filled.Close, stringResource(R.string.lyrics_close)) { onClose() }
                    ToolButton(Icons.Filled.NavigateBefore, stringResource(R.string.lyrics_prev)) { st.pageCommands.tryEmit(-1) }
                    ToolButton(Icons.Filled.NavigateNext, stringResource(R.string.lyrics_next)) { st.pageCommands.tryEmit(1) }
                    ToolButton(Icons.Filled.Remove, stringResource(R.string.lyrics_slower)) { st.speed = (st.speed - 1).coerceAtLeast(0) }
                    ToolButton(if (st.speed > 0) Icons.Filled.Pause else Icons.Filled.PlayArrow, stringResource(R.string.lyrics_auto), tint = Orange) {
                        st.speed = if (st.speed > 0) 0 else 3
                    }
                    ToolButton(Icons.Filled.Add, stringResource(R.string.lyrics_faster)) { st.speed = (st.speed + 1).coerceAtMost(10) }
                    Text("${st.speed}", color = TextHi, fontSize = 16.sp, modifier = Modifier.padding(horizontal = 6.dp))
                    if (!doc.isPdf) {
                        ToolButton(Icons.Filled.TextDecrease, stringResource(R.string.lyrics_smaller)) { st.fontSp = (st.fontSp - 4f).coerceAtLeast(20f) }
                        ToolButton(Icons.Filled.TextIncrease, stringResource(R.string.lyrics_bigger)) { st.fontSp = (st.fontSp + 4f).coerceAtMost(120f) }
                    }
                    ToolButton(Icons.Filled.Contrast, stringResource(R.string.lyrics_contrast)) { st.theme = (st.theme + 1) % Themes.size }
                }
            }
        }
    }
}

/** Avance automático: 1..10 => 12..120 dp por segundo. */
private fun pxPerSecond(speed: Int, density: Float) = speed * 12f * density

@Composable
private fun TextLyrics(file: File, theme: LyricsTheme, st: PrompterState) {
    val text = remember(file) { decodeText(file.readBytes()) }
    val scroll = rememberScrollState()
    val density = LocalDensity.current.density
    BoxWithConstraints(Modifier.fillMaxSize()) {
        val pageH = with(LocalDensity.current) { maxHeight.toPx() } * 0.85f
        Text(
            text, color = theme.fg, fontSize = st.fontSp.sp, lineHeight = (st.fontSp * 1.35f).sp, fontWeight = FontWeight.SemiBold,
            modifier = Modifier.fillMaxSize().verticalScroll(scroll).padding(horizontal = 32.dp, vertical = 48.dp).padding(bottom = 160.dp),
        )
        LaunchedEffect(st) {
            st.pageCommands.collect { c ->
                when {
                    c == 1 -> scroll.animateScrollBy(pageH)
                    c == -1 -> scroll.animateScrollBy(-pageH)
                    c >= 1000 -> scroll.animateScrollTo(((c - 1001).coerceAtLeast(0) * pageH).toInt())
                }
            }
        }
    }
    AutoScroll(st, density) { scroll.scrollBy(it) }
}

/** Avance continuo sincronizado con los frames de la pantalla. */
@Composable
private fun AutoScroll(st: PrompterState, density: Float, by: suspend (Float) -> Unit) {
    LaunchedEffect(st.speed) {
        if (st.speed <= 0) return@LaunchedEffect
        var last = withFrameNanos { it }
        var acc = 0f
        while (true) {
            val now = withFrameNanos { it }
            acc += pxPerSecond(st.speed, density) * (now - last) / 1e9f
            last = now
            if (acc >= 1f) {
                val d = acc
                acc = 0f
                by(d)
            }
        }
    }
}

@Composable
private fun PdfPages(file: File, theme: LyricsTheme, st: PrompterState) {
    val pfd = remember(file) { ParcelFileDescriptor.open(file, ParcelFileDescriptor.MODE_READ_ONLY) }
    val renderer = remember(pfd) { runCatching { PdfRenderer(pfd) }.getOrNull() }
    val lock = remember { Mutex() }
    DisposableEffect(renderer) { onDispose { runCatching { renderer?.close() }; runCatching { pfd.close() } } }
    if (renderer == null) {
        Text(stringResource(R.string.lyrics_bad_pdf), color = theme.fg, modifier = Modifier.padding(32.dp))
        return
    }
    val list = rememberLazyListState()
    val density = LocalDensity.current.density
    val invert = ColorFilter.colorMatrix(ColorMatrix(floatArrayOf(-1f, 0f, 0f, 0f, 255f, 0f, -1f, 0f, 0f, 255f, 0f, 0f, -1f, 0f, 255f, 0f, 0f, 0f, 1f, 0f)))
    BoxWithConstraints(Modifier.fillMaxSize()) {
        val widthPx = with(LocalDensity.current) { maxWidth.toPx() }.toInt().coerceIn(320, 2400)
        LazyColumn(state = list, modifier = Modifier.fillMaxSize(), contentPadding = PaddingValues(bottom = 160.dp)) {
            itemsIndexed(List(renderer.pageCount) { it }) { _, index ->
                var bmp by remember(index, widthPx) { mutableStateOf<Bitmap?>(null) }
                LaunchedEffect(index, widthPx) {
                    bmp = withContext(Dispatchers.IO) {
                        lock.withLock {
                            // PdfRenderer sólo permite una página abierta a la vez.
                            renderer.openPage(index).use { p ->
                                val h = (widthPx.toFloat() * p.height / p.width).toInt().coerceAtLeast(1)
                                val b = Bitmap.createBitmap(widthPx, h, Bitmap.Config.ARGB_8888)
                                b.eraseColor(AColor.WHITE)
                                p.render(b, null, null, PdfRenderer.Page.RENDER_MODE_FOR_DISPLAY)
                                b
                            }
                        }
                    }
                }
                val b = bmp
                if (b != null) {
                    Image(b.asImageBitmap(), null, contentScale = ContentScale.FillWidth, modifier = Modifier.fillMaxWidth(),
                        colorFilter = if (theme.invertPdf) invert else null)
                } else {
                    Spacer(Modifier.fillMaxWidth().height(400.dp))
                }
                Spacer(Modifier.height(8.dp))
            }
        }
    }
    LaunchedEffect(st) {
        st.pageCommands.collect { c ->
            val n = renderer.pageCount
            when {
                c == 1 -> list.animateScrollToItem((list.firstVisibleItemIndex + 1).coerceAtMost(n - 1))
                c == -1 -> list.animateScrollToItem((list.firstVisibleItemIndex - 1).coerceAtLeast(0))
                c >= 1000 -> list.animateScrollToItem((c - 1001).coerceIn(0, n - 1))
            }
        }
    }
    AutoScroll(st, density) { list.scrollBy(it) }
}
