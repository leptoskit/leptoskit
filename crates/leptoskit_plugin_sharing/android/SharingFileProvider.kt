package dev.leptoskit.plugins.sharing

import androidx.core.content.FileProvider

/**
 * A distinct class, so the manifest can declare this provider beside any
 * FileProvider the app or another library declares. It serves only
 * `cacheDir/leptoskit-sharing`, through `res/xml/leptoskit_sharing_paths.xml`.
 */
class SharingFileProvider : FileProvider()
