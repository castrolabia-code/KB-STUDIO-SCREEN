package com.castrolabs.kbstudioscreen

import android.app.Activity
import android.content.Context
import android.media.MediaCodecList
import android.media.MediaFormat
import android.os.Build
import android.util.DisplayMetrics

/** Datos REALES de la pantalla y del decodificador (se negocian con la PC, nunca se suponen). */
data class DeviceInfo(
    val widthPx: Int, val heightPx: Int, val dpi: Int, val refresh: Int,
    val isTablet: Boolean, val decMaxW: Int, val decMaxH: Int, val decoderName: String,
) {
    companion object {
        fun detect(a: Activity): DeviceInfo {
            val (w, h) = if (Build.VERSION.SDK_INT >= 30) {
                val b = a.windowManager.maximumWindowMetrics.bounds
                b.width() to b.height()
            } else {
                val m = DisplayMetrics()
                @Suppress("DEPRECATION") a.windowManager.defaultDisplay.getRealMetrics(m)
                m.widthPixels to m.heightPixels
            }
            @Suppress("DEPRECATION")
            val refresh = a.windowManager.defaultDisplay.refreshRate.toInt().coerceIn(30, 240)
            val dpi = a.resources.displayMetrics.densityDpi
            val tablet = a.resources.configuration.smallestScreenWidthDp >= 600
            val (dw, dh, name) = decoderLimits(a)
            return DeviceInfo(w, h, dpi, refresh, tablet, dw, dh, name)
        }

        /** Mayor tamaño que el decodificador H.264 (preferentemente por hardware) soporta. */
        private fun decoderLimits(ctx: Context): Triple<Int, Int, String> {
            val list = MediaCodecList(MediaCodecList.REGULAR_CODECS).codecInfos
                .filter { !it.isEncoder && it.supportedTypes.any { t -> t.equals(MediaFormat.MIMETYPE_VIDEO_AVC, true) } }
            val best = list.firstOrNull { it.isHardwareAccelerated } ?: list.firstOrNull()
                ?: return Triple(1280, 720, "ninguno")
            val caps = best.getCapabilitiesForType(MediaFormat.MIMETYPE_VIDEO_AVC).videoCapabilities
            val candidates = listOf(3840 to 2160, 2560 to 1600, 2560 to 1440, 1920 to 1200, 1920 to 1080, 1280 to 800, 1280 to 720)
            val ok = candidates.firstOrNull { (cw, ch) -> caps.isSizeSupported(cw, ch) } ?: (1280 to 720)
            return Triple(ok.first, ok.second, best.name)
        }
    }
}
