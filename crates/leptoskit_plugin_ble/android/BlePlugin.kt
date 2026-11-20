package dev.leptoskit.plugins.ble

import android.Manifest
import android.annotation.SuppressLint
import android.bluetooth.BluetoothAdapter
import android.bluetooth.BluetoothGatt
import android.bluetooth.BluetoothGattCallback
import android.bluetooth.BluetoothGattCharacteristic
import android.bluetooth.BluetoothGattDescriptor
import android.bluetooth.BluetoothManager
import android.bluetooth.BluetoothProfile
import android.bluetooth.le.ScanCallback
import android.bluetooth.le.ScanFilter
import android.bluetooth.le.ScanResult
import android.bluetooth.le.ScanSettings
import android.content.Context
import android.content.pm.PackageManager
import android.os.Build
import android.os.ParcelUuid
import androidx.core.content.ContextCompat
import dev.leptoskit.bridge.NativePlugin
import dev.leptoskit.bridge.PluginException
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.delay
import java.io.ByteArrayOutputStream
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.util.ArrayDeque
import java.util.UUID
import java.util.concurrent.ConcurrentHashMap

/**
 * BLE GATT central plugin for Android phones and Wear OS watches.
 */
@SuppressLint("MissingPermission")
class BlePlugin(
    private val context: Context,
) : NativePlugin {
    override val name = "ble"
    override val methods = intArrayOf(0, 1, 2, 3, 4, 5, 6, 7)

    private val manager = context.getSystemService(Context.BLUETOOTH_SERVICE) as? BluetoothManager
    private val connections = ConcurrentHashMap<String, BluetoothGatt>()
    private val lock = Any()
    private val notifications = ArrayDeque<ByteArray>()
    private var dropped = 0
    private var waiter: CompletableDeferred<ByteArray>? = null
    private var closed = false

    override suspend fun invoke(method: Int, payload: ByteBuffer): ByteArray {
        if (closed) throw PluginException(PluginException.CANCELLED)
        return when (method) {
            0, 1 -> encodeU32(availabilityCode())
            2 -> scan(payload)
            3 -> connect(readSingleString(payload))
            4 -> disconnect(readSingleString(payload))
            5 -> setNotify(payload, true)
            6 -> setNotify(payload, false)
            7 -> nextNotifications()
            else -> throw PluginException(PluginException.UNAVAILABLE)
        }
    }

    override fun close() {
        closed = true
        for (gatt in connections.values) {
            runCatching { gatt.close() }
        }
        connections.clear()
        synchronized(lock) {
            waiter?.cancel()
            waiter = null
        }
    }

    private fun availabilityCode(): Int {
        val adapter: BluetoothAdapter = manager?.adapter ?: return 3
        if (!hasPermissions()) return 2
        return if (adapter.isEnabled) 0 else 1
    }

    private fun hasPermissions(): Boolean {
        if (Build.VERSION.SDK_INT < 31) return true
        val scanOk = ContextCompat.checkSelfPermission(context, Manifest.permission.BLUETOOTH_SCAN) ==
            PackageManager.PERMISSION_GRANTED
        val connOk = ContextCompat.checkSelfPermission(context, Manifest.permission.BLUETOOTH_CONNECT) ==
            PackageManager.PERMISSION_GRANTED
        return scanOk && connOk
    }

    private suspend fun scan(payload: ByteBuffer): ByteArray {
        if (availabilityCode() != 0) throw PluginException(PluginException.UNAVAILABLE)
        val reader = WireReader(payload)
        val timeoutMs = reader.u32().coerceIn(500, 30_000)
        val filterCount = reader.u32()
        val uuids = List(filterCount) { reader.string() }
        reader.finished()

        val scanner = manager?.adapter?.bluetoothLeScanner ?: throw PluginException(PluginException.UNAVAILABLE)
        val found = ConcurrentHashMap<String, ScanResult>()
        val callback =
            object : ScanCallback() {
                override fun onScanResult(callbackType: Int, result: ScanResult) {
                    found[result.device.address] = result
                }
            }
        val filters = uuids.mapNotNull { runCatching { ScanFilter.Builder().setServiceUuid(ParcelUuid(UUID.fromString(it))).build() }.getOrNull() }
        val settings = ScanSettings.Builder().setScanMode(ScanSettings.SCAN_MODE_LOW_LATENCY).build()
        scanner.startScan(filters, settings, callback)
        try {
            delay(timeoutMs.toLong())
        } finally {
            runCatching { scanner.stopScan(callback) }
        }
        return encodeDevices(found.values.toList())
    }

    private suspend fun connect(deviceId: String): ByteArray {
        if (availabilityCode() != 0) throw PluginException(PluginException.UNAVAILABLE)
        val adapter = manager?.adapter ?: throw PluginException(PluginException.UNAVAILABLE)
        val device = adapter.getRemoteDevice(deviceId)
        val ready = CompletableDeferred<Unit>()
        val gatt =
            device.connectGatt(
                context,
                false,
                object : BluetoothGattCallback() {
                    override fun onConnectionStateChange(g: BluetoothGatt, status: Int, newState: Int) {
                        if (newState == BluetoothProfile.STATE_CONNECTED) {
                            g.discoverServices()
                        } else if (newState == BluetoothProfile.STATE_DISCONNECTED) {
                            connections.remove(deviceId)
                            if (!ready.isCompleted) ready.completeExceptionally(PluginException(PluginException.NATIVE))
                        }
                    }

                    override fun onServicesDiscovered(g: BluetoothGatt, status: Int) {
                        if (status == BluetoothGatt.GATT_SUCCESS) {
                            ready.complete(Unit)
                        } else if (!ready.isCompleted) {
                            ready.completeExceptionally(PluginException(PluginException.NATIVE))
                        }
                    }

                    override fun onCharacteristicChanged(
                        g: BluetoothGatt,
                        characteristic: BluetoothGattCharacteristic,
                        value: ByteArray,
                    ) {
                        enqueueNotification(
                            deviceId,
                            characteristic.service.uuid.toString().lowercase(),
                            characteristic.uuid.toString().lowercase(),
                            value,
                        )
                    }

                    @Deprecated("Android < 33 callback")
                    override fun onCharacteristicChanged(
                        g: BluetoothGatt,
                        characteristic: BluetoothGattCharacteristic,
                    ) {
                        @Suppress("DEPRECATION")
                        val value = characteristic.value ?: return
                        enqueueNotification(
                            deviceId,
                            characteristic.service.uuid.toString().lowercase(),
                            characteristic.uuid.toString().lowercase(),
                            value,
                        )
                    }
                },
            )
        connections[deviceId] = gatt
        ready.await()
        return ByteArray(0)
    }

    private fun disconnect(deviceId: String): ByteArray {
        connections.remove(deviceId)?.close()
        return ByteArray(0)
    }

    private fun setNotify(payload: ByteBuffer, enable: Boolean): ByteArray {
        val reader = WireReader(payload)
        val deviceId = reader.string()
        val serviceUuid = UUID.fromString(reader.string())
        val charUuid = UUID.fromString(reader.string())
        reader.finished()

        val gatt = connections[deviceId] ?: throw PluginException(PluginException.UNAVAILABLE)
        val characteristic =
            gatt.getService(serviceUuid)?.getCharacteristic(charUuid)
                ?: throw PluginException(PluginException.UNAVAILABLE)
        gatt.setCharacteristicNotification(characteristic, enable)
        val cccd = characteristic.getDescriptor(CCCD_UUID)
        if (cccd != null) {
            val value =
                if (enable) {
                    BluetoothGattDescriptor.ENABLE_NOTIFICATION_VALUE
                } else {
                    BluetoothGattDescriptor.DISABLE_NOTIFICATION_VALUE
                }
            if (Build.VERSION.SDK_INT >= 33) {
                gatt.writeDescriptor(cccd, value)
            } else {
                @Suppress("DEPRECATION")
                cccd.value = value
                @Suppress("DEPRECATION")
                gatt.writeDescriptor(cccd)
            }
        }
        return ByteArray(0)
    }

    private suspend fun nextNotifications(): ByteArray {
        val ready = drainLocked()
        if (ready != null) return ready
        val pending = CompletableDeferred<ByteArray>()
        synchronized(lock) {
            val again = drainLocked()
            if (again != null) return again
            waiter = pending
        }
        return pending.await()
    }

    private fun enqueueNotification(deviceId: String, service: String, charUuid: String, value: ByteArray) {
        val out = ByteArrayOutputStream()
        writeBytes(out, deviceId.toByteArray(Charsets.UTF_8))
        writeBytes(out, service.toByteArray(Charsets.UTF_8))
        writeBytes(out, charUuid.toByteArray(Charsets.UTF_8))
        writeI64(out, System.currentTimeMillis())
        writeBytes(out, value)
        val encoded = out.toByteArray()
        synchronized(lock) {
            if (notifications.size >= 128) {
                notifications.removeFirst()
                dropped += 1
            }
            notifications.addLast(encoded)
            val w = waiter
            if (w != null) {
                waiter = null
                val batch = drainLocked()
                if (batch != null) w.complete(batch)
            }
        }
    }

    private fun drainLocked(): ByteArray? =
        synchronized(lock) {
            if (notifications.isEmpty() && dropped == 0) return@synchronized null
            val out = ByteArrayOutputStream()
            writeU32(out, dropped)
            writeU32(out, notifications.size)
            dropped = 0
            while (notifications.isNotEmpty()) out.write(notifications.removeFirst())
            out.toByteArray()
        }

    private fun readSingleString(payload: ByteBuffer): String {
        val reader = WireReader(payload)
        val value = reader.string()
        reader.finished()
        return value
    }

    private companion object {
        val CCCD_UUID: UUID = UUID.fromString("00002902-0000-1000-8000-00805f9b34fb")
    }
}

private class WireReader(payload: ByteBuffer) {
    private val buf = payload.duplicate().order(ByteOrder.LITTLE_ENDIAN)

    fun u32(): Int {
        if (buf.remaining() < 4) throw PluginException(PluginException.INVALID_ARGUMENT)
        return buf.int
    }

    fun string(): String {
        val len = u32()
        if (len < 0 || len > buf.remaining()) throw PluginException(PluginException.INVALID_ARGUMENT)
        val bytes = ByteArray(len)
        buf.get(bytes)
        return String(bytes, Charsets.UTF_8)
    }

    fun finished() {
        if (buf.hasRemaining()) throw PluginException(PluginException.INVALID_ARGUMENT)
    }
}

private fun encodeDevices(results: List<ScanResult>): ByteArray {
    val out = ByteArrayOutputStream()
    writeU32(out, results.size)
    for (r in results) {
        writeBytes(out, r.device.address.toByteArray(Charsets.UTF_8))
        val name = r.device.name ?: r.scanRecord?.deviceName
        if (name == null) {
            writeU32(out, -1)
        } else {
            writeBytes(out, name.toByteArray(Charsets.UTF_8))
        }
        writeU32(out, r.rssi)
        val uuids = r.scanRecord?.serviceUuids.orEmpty()
        writeU32(out, uuids.size)
        for (u in uuids) {
            writeBytes(out, u.uuid.toString().lowercase().toByteArray(Charsets.UTF_8))
        }
    }
    return out.toByteArray()
}

private fun encodeU32(value: Int): ByteArray =
    ByteBuffer.allocate(4).order(ByteOrder.LITTLE_ENDIAN).putInt(value).array()

private fun writeBytes(out: ByteArrayOutputStream, bytes: ByteArray) {
    writeU32(out, bytes.size)
    out.write(bytes)
}

private fun writeU32(out: ByteArrayOutputStream, value: Int) {
    out.write(value and 0xFF)
    out.write((value shr 8) and 0xFF)
    out.write((value shr 16) and 0xFF)
    out.write((value shr 24) and 0xFF)
}

private fun writeI64(out: ByteArrayOutputStream, value: Long) {
    for (shift in 0 until 64 step 8) {
        out.write(((value shr shift) and 0xFF).toInt())
    }
}
