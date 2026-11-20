package com.castrolabs.kbstudioscreen

import android.content.Context
import android.net.nsd.NsdManager
import android.net.nsd.NsdServiceInfo
import android.net.wifi.WifiManager
import java.util.concurrent.ConcurrentLinkedQueue
import java.util.concurrent.atomic.AtomicBoolean

data class FoundHost(val name: String, val host: String, val port: Int, val fp: String?)

/**
 * Busca PCs con kbss-engine por mDNS/DNS-SD (`_kbss._udp`).
 * En algunos fabricantes NsdManager falla: por eso el QR y la IP manual
 * existen siempre como alternativa.
 */
class Discovery(ctx: Context, private val onChange: (List<FoundHost>) -> Unit) {
    private val nsd = ctx.getSystemService(Context.NSD_SERVICE) as NsdManager
    private val wifi = ctx.applicationContext.getSystemService(Context.WIFI_SERVICE) as WifiManager
    private val lock = wifi.createMulticastLock("kbss-discovery").apply { setReferenceCounted(false) }
    private val found = LinkedHashMap<String, FoundHost>()
    private val queue = ConcurrentLinkedQueue<NsdServiceInfo>()
    private val resolving = AtomicBoolean(false)
    private var listener: NsdManager.DiscoveryListener? = null

    fun start() {
        if (listener != null) return
        lock.acquire()
        val l = object : NsdManager.DiscoveryListener {
            override fun onDiscoveryStarted(t: String) {}
            override fun onDiscoveryStopped(t: String) {}
            override fun onStartDiscoveryFailed(t: String, e: Int) {}
            override fun onStopDiscoveryFailed(t: String, e: Int) {}
            override fun onServiceFound(s: NsdServiceInfo) {
                queue.add(s)
                resolveNext()
            }
            override fun onServiceLost(s: NsdServiceInfo) {
                synchronized(found) { found.remove(s.serviceName) }
                publish()
            }
        }
        listener = l
        nsd.discoverServices("_kbss._udp.", NsdManager.PROTOCOL_DNS_SD, l)
    }

    // NsdManager (API < 34) sólo resuelve un servicio a la vez.
    private fun resolveNext() {
        if (!resolving.compareAndSet(false, true)) return
        val s = queue.poll() ?: run { resolving.set(false); return }
        @Suppress("DEPRECATION")
        nsd.resolveService(s, object : NsdManager.ResolveListener {
            override fun onResolveFailed(si: NsdServiceInfo, e: Int) {
                resolving.set(false); resolveNext()
            }
            override fun onServiceResolved(si: NsdServiceInfo) {
                @Suppress("DEPRECATION")
                val addr = si.host?.hostAddress
                if (addr != null && !addr.contains(':')) {
                    val fp = si.attributes["fp"]?.let { String(it) }
                    val name = si.attributes["name"]?.let { String(it) } ?: si.serviceName
                    synchronized(found) { found[si.serviceName] = FoundHost(name, addr, si.port, fp) }
                    publish()
                }
                resolving.set(false); resolveNext()
            }
        })
    }

    private fun publish() = onChange(synchronized(found) { found.values.toList() })

    fun stop() {
        listener?.let { runCatching { nsd.stopServiceDiscovery(it) } }
        listener = null
        if (lock.isHeld) lock.release()
    }
}

/** Traduce "codigo|detalle" del núcleo a un mensaje claro y localizado. */
fun errorMessage(ctx: Context, e: Throwable): String {
    val raw = e.message ?: ""
    val code = raw.substringBefore('|')
    val id = when (code) {
        "unreachable", "io", "closed" -> R.string.err_unreachable
        "slots_full" -> R.string.err_slots_full
        "pin_wrong" -> R.string.err_pin_wrong
        "pin_expired" -> R.string.err_pin_expired
        "too_many" -> R.string.err_too_many
        "rejected" -> R.string.err_rejected
        "mitm" -> R.string.err_mitm
        "not_paired" -> R.string.err_not_paired
        "version" -> R.string.err_version
        "qr_invalid" -> R.string.err_qr_invalid
        else -> return ctx.getString(R.string.err_generic, raw.substringAfter('|'))
    }
    return ctx.getString(id)
}
