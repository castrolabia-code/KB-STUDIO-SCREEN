package com.castrolabs.kbstudioscreen

import android.graphics.ImageFormat
import android.media.ImageReader
import android.os.Handler
import android.os.HandlerThread
import android.util.Base64
import androidx.test.core.app.ActivityScenario
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Assume.assumeTrue
import org.junit.Test
import org.junit.runner.RunWith
import java.util.concurrent.atomic.AtomicInteger

/**
 * Prueba extremo a extremo en un Android real (emulador de CI) contra el
 * host de demostración (`demo_host`): núcleo Rust por JNI, emparejamiento QR
 * sobre QUIC, decodificación H.264 con MediaCodec y envío de toques.
 * Recibe el QR en base64 con `-e kbssQrB64 ...`; sin él, se omite.
 */
@RunWith(AndroidJUnit4::class)
class EndToEndTest {

    @Test
    fun mainScreenStartsWithoutCrashing() {
        ActivityScenario.launch(MainActivity::class.java).use { s ->
            s.onActivity { a -> assertFalse(a.isFinishing) }
        }
    }

    @Test
    fun qrPairingVideoDecodeAndTouch() {
        val b64 = InstrumentationRegistry.getArguments().getString("kbssQrB64")
        assumeTrue("sin host de demostración", b64 != null)
        val qr = String(Base64.decode(b64, Base64.DEFAULT)).trim()

        val id = NativeCore.generateIdentity()
        val h = NativeCore.connect("", 0, NativeCore.MODE_QR, "", qr, id[0], id[1], "Emulador CI", true,
            1280, 800, 213, 60, 1920, 1080)
        val st = JSONObject(NativeCore.stage(h))
        assertEquals("ready", st.getString("stage"))
        assertEquals("control", st.getString("permission"))

        // Decodificación real con MediaCodec hacia una Surface (ImageReader).
        val thread = HandlerThread("img").apply { start() }
        val images = AtomicInteger(0)
        val reader = ImageReader.newInstance(1280, 720, ImageFormat.YUV_420_888, 4)
        reader.setOnImageAvailableListener({ r -> r.acquireLatestImage()?.let { it.close(); images.incrementAndGet() } }, Handler(thread.looper))
        val dec = H264Decoder(reader.surface, { _, _ -> }) { NativeCore.requestKeyframe(h) }
        val end = System.currentTimeMillis() + 5000
        var frames = 0
        while (System.currentTimeMillis() < end) {
            val f = NativeCore.nextFrame(h) ?: break
            dec.feed(f)
            frames++
        }
        Thread.sleep(300)
        val rendered = dec.rendered.get()
        dec.release()
        println("E2E frames=$frames rendered=$rendered images=${images.get()}")
        assertTrue("frames recibidos: $frames", frames >= 30)
        assertTrue("frames decodificados: $rendered", rendered >= 10)
        assertTrue("imágenes producidas: ${images.get()}", images.get() >= 5)

        // Toques: la barra negra no se envía; el arrastre sí.
        NativeCore.setViewport(h, 1280f, 800f, 1280, 720, 1f, 0f, 0f)
        assertFalse(NativeCore.touch(h, NativeCore.PHASE_DOWN, 640f, 10f))
        assertTrue(NativeCore.touch(h, NativeCore.PHASE_DOWN, 640f, 400f))
        assertTrue(NativeCore.touch(h, NativeCore.PHASE_MOVE, 640f, 200f))
        assertTrue(NativeCore.touch(h, NativeCore.PHASE_UP, 640f, 200f))
        Thread.sleep(500)
        NativeCore.close(h)
        reader.close()
        thread.quitSafely()
    }
}
