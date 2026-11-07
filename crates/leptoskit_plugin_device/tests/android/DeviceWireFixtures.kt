package dev.leptoskit.plugins.device

/** The same fixture and bytes as `android_fixture` in the Rust crate's tests. */
internal val ANDROID_FIXTURE =
    DeviceFacts(
        kind = 1,
        simulator = null,
        totalMemory = 12L shl 30,
        availableMemory = 3L shl 30,
        totalDisk = 256L shl 30,
        freeDisk = 100L shl 30,
        cpuCount = 8,
        manufacturer = "Google",
        brand = "google",
        model = "Pixel 9",
        osName = "Android",
        osVersion = "15",
        architecture = "arm64-v8a",
        kernelVersion = "6.1.75",
        build =
            AndroidBuild(
                sdkInt = 35,
                previewSdkInt = 0,
                lowRamDevice = false,
                lowMemory = true,
                buildTime = 1_700_000_000_123L,
                baseOs = null,
                codename = "REL",
                incremental = "12345",
                release = "15",
                securityPatch = "2026-10-05",
                board = "tokay",
                bootloader = "b1",
                device = "tokay",
                display = "AP4A.1",
                fingerprint = "google/tokay/tokay:15/AP4A.1/12345:user/release-keys",
                hardware = "tokay",
                host = "abfarm",
                id = "AP4A.1",
                product = "tokay",
                tags = "release-keys",
                type = "user",
                supportedAbis = listOf("arm64-v8a"),
                supported32BitAbis = emptyList(),
                supported64BitAbis = listOf("arm64-v8a"),
                systemFeatures = listOf("android.hardware.camera", "android.hardware.wifi"),
            ),
    )

internal val ANDROID_GOLDEN =
    listOf(
        "020000000100000000000000000000000300000001000000000000c0000000000000000040000000",
        "0100000000000000190000000800000006000000476f6f676c6506000000676f6f676c6507000000",
        "506978656c20390000000007000000416e64726f69640200000031350900000061726d36342d7638",
        "6106000000362e312e3735020000002300000001000000000000000100000002000000010000007b",
        "68e5cf8b010000000000000300000052454c0500000031323334350200000031350a000000323032",
        "362d31302d303505000000746f6b617902000000623105000000746f6b617906000000415034412e",
        "3134000000676f6f676c652f746f6b61792f746f6b61793a31352f415034412e312f31323334353a",
        "757365722f72656c656173652d6b65797305000000746f6b61790600000061626661726d06000000",
        "415034412e3105000000746f6b61790c00000072656c656173652d6b657973040000007573657201",
        "0000000900000061726d36342d76386100000000010000000900000061726d36342d763861020000",
        "0017000000616e64726f69642e68617264776172652e63616d65726115000000616e64726f69642e",
        "68617264776172652e77696669",
    ).joinToString("")

/** The same identifiers and bytes as `identifiers_fixture` in the Rust tests. */
internal val IDENTIFIERS_FIXTURE =
    DeviceIdentifiers(
        deviceName = "Ada's iPhone",
        vendorId = "01234567-89AB-CDEF-0123-456789ABCDEF",
        machineId = null,
    )

internal val IDENTIFIERS_GOLDEN =
    listOf(
        "010000000c0000004164612773206950686f6e652400000030313233343536372d383941422d4344",
        "45462d303132332d34353637383941424344454600000000",
    ).joinToString("")

internal fun ByteArray.hex(): String = joinToString("") { "%02x".format(it) }
