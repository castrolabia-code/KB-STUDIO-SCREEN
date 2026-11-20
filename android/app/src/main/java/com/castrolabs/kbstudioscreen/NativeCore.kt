package com.castrolabs.kbstudioscreen

/**
 * Puente con libkbcore.so (núcleo Rust compartido con la PC).
 * Las funciones marcadas "bloquea" deben llamarse fuera del hilo de UI.
 * Los errores llegan como IllegalStateException("codigo|detalle").
 */
object NativeCore {
    init {
        System.loadLibrary("kbcore")
    }

    const val MODE_TRUSTED = 0
    const val MODE_QR = 1
    const val MODE_PIN = 2

    const val PHASE_DOWN = 0
    const val PHASE_MOVE = 1
    const val PHASE_UP = 2
    const val PHASE_CANCEL = 3

    /** Devuelve [certificadoDER, clavePKCS8]. */
    @JvmStatic external fun generateIdentity(): Array<ByteArray>

    /** Bloquea. Devuelve un handle (> 0). */
    @JvmStatic external fun connect(
        host: String, port: Int, mode: Int, pinnedFp: String, qrUri: String,
        cert: ByteArray, key: ByteArray, deviceName: String, isTablet: Boolean,
        screenW: Int, screenH: Int, dpi: Int, refresh: Int, decMaxW: Int, decMaxH: Int,
    ): Long

    @JvmStatic external fun stage(h: Long): String
    @JvmStatic external fun hostInfo(h: Long): String
    /** Bloquea. Pide a la PC que muestre un PIN; devuelve su vigencia en segundos. */
    @JvmStatic external fun requestPin(h: Long): Int
    /** Bloquea hasta que la PC apruebe o rechace. */
    @JvmStatic external fun submitPin(h: Long, pin: String): String
    /** Bloquea. Cabecera de 21 bytes + H.264 Annex-B, o null si se cerró. */
    @JvmStatic external fun nextFrame(h: Long): ByteArray?
    /** Bloquea. Evento de control en JSON. */
    @JvmStatic external fun nextControl(h: Long): String
    @JvmStatic external fun setViewport(h: Long, viewW: Float, viewH: Float, frameW: Int, frameH: Int, zoom: Float, offX: Float, offY: Float)
    @JvmStatic external fun touch(h: Long, phase: Int, x: Float, y: Float): Boolean
    @JvmStatic external fun wheel(h: Long, x: Float, y: Float, dy: Int)
    @JvmStatic external fun requestKeyframe(h: Long)
    @JvmStatic external fun selectMonitor(h: Long, id: String)
    @JvmStatic external fun monitors(h: Long): String
    @JvmStatic external fun close(h: Long)
}

/** Sesión activa compartida entre pantallas (un solo host por app). */
object SessionHolder {
    @Volatile var handle: Long = 0
    @Volatile var host: SavedHost? = null
}
