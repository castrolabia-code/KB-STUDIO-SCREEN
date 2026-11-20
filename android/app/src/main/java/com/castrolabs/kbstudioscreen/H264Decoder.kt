package com.castrolabs.kbstudioscreen

import android.media.MediaCodec
import android.media.MediaFormat
import android.os.Build
import android.view.Surface
import java.nio.ByteBuffer
import java.util.concurrent.atomic.AtomicInteger

/**
 * Decodificación H.264 por hardware directo a la Surface (sin copias).
 * Recibe "cabecera de 21 bytes + Annex-B" desde NativeCore.nextFrame.
 * Si se pierde un frame se espera el siguiente keyframe y se lo pide a la PC.
 */
class H264Decoder(
    private val surface: Surface,
    private val onSize: (Int, Int) -> Unit,
    private val requestKeyframe: () -> Unit,
) {
    private var codec: MediaCodec? = null
    private var width = 0
    private var height = 0
    private var needKey = true
    @Volatile private var running = true
    private var outThread: Thread? = null
    val rendered = AtomicInteger(0)
    @Volatile var lastLatencyHintMs = 0L

    fun feed(buf: ByteArray) {
        if (buf.size <= HEADER) return
        val bb = ByteBuffer.wrap(buf)
        bb.int // seq
        val captureUs = bb.long
        val key = bb.get().toInt() == 1
        val w = bb.short.toInt() and 0xFFFF
        val h = bb.short.toInt() and 0xFFFF
        val len = bb.int
        if (len <= 0 || HEADER + len > buf.size) return

        if (codec == null || w != width || h != height) {
            if (!key) { requestKeyframe(); return }
            configure(w, h, buf, HEADER, len)
        }
        if (needKey && !key) return
        val c = codec ?: return
        try {
            val idx = c.dequeueInputBuffer(20_000)
            if (idx < 0) {
                // Decodificador saturado (tablet lenta): descartar y resincronizar.
                needKey = true; requestKeyframe(); return
            }
            val ib = c.getInputBuffer(idx) ?: return
            ib.clear()
            ib.put(buf, HEADER, len)
            c.queueInputBuffer(idx, 0, len, captureUs, if (key) MediaCodec.BUFFER_FLAG_KEY_FRAME else 0)
            needKey = false
        } catch (e: IllegalStateException) {
            release(); needKey = true; requestKeyframe()
        }
    }

    private fun configure(w: Int, h: Int, data: ByteArray, off: Int, len: Int) {
        release()
        val (sps, pps) = extractSpsPps(data, off, len)
        val f = MediaFormat.createVideoFormat(MediaFormat.MIMETYPE_VIDEO_AVC, w, h)
        sps?.let { f.setByteBuffer("csd-0", ByteBuffer.wrap(it)) }
        pps?.let { f.setByteBuffer("csd-1", ByteBuffer.wrap(it)) }
        if (Build.VERSION.SDK_INT >= 30) f.setInteger(MediaFormat.KEY_LOW_LATENCY, 1)
        f.setInteger(MediaFormat.KEY_PRIORITY, 0) // tiempo real
        val c = MediaCodec.createDecoderByType(MediaFormat.MIMETYPE_VIDEO_AVC)
        c.configure(f, surface, null, 0)
        c.start()
        codec = c
        width = w; height = h
        running = true
        onSize(w, h)
        outThread = Thread({ drain(c) }, "kbss-decode-out").also { it.start() }
    }

    private fun drain(c: MediaCodec) {
        val info = MediaCodec.BufferInfo()
        while (running && codec === c) {
            try {
                val i = c.dequeueOutputBuffer(info, 20_000)
                if (i >= 0) {
                    // Renderizar ya: priorizamos latencia sobre suavidad.
                    c.releaseOutputBuffer(i, true)
                    rendered.incrementAndGet()
                }
            } catch (_: IllegalStateException) {
                break
            }
        }
    }

    fun release() {
        running = false
        val c = codec ?: return
        codec = null
        outThread?.join(200)
        runCatching { c.stop() }
        runCatching { c.release() }
    }

    companion object {
        const val HEADER = 21

        /** Devuelve (SPS, PPS) con código de inicio, buscando en el Annex-B. */
        fun extractSpsPps(d: ByteArray, off: Int, len: Int): Pair<ByteArray?, ByteArray?> {
            val starts = ArrayList<Int>()
            var i = off
            val end = off + len
            while (i + 3 < end) {
                if (d[i].toInt() == 0 && d[i + 1].toInt() == 0 && d[i + 2].toInt() == 1) { starts.add(i + 3); i += 3 } else i++
            }
            var sps: ByteArray? = null
            var pps: ByteArray? = null
            for ((k, s) in starts.withIndex()) {
                var e = if (k + 1 < starts.size) starts[k + 1] - 3 else end
                while (e > s && d[e - 1].toInt() == 0) e-- // quitar el 0 extra de códigos de 4 bytes
                val type = d[s].toInt() and 0x1F
                val nal = byteArrayOf(0, 0, 0, 1) + d.copyOfRange(s, e)
                if (type == 7 && sps == null) sps = nal
                if (type == 8 && pps == null) pps = nal
            }
            return sps to pps
        }
    }
}
