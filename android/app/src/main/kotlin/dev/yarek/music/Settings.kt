package dev.yarek.music

import android.content.Context

/** Small preferences (SharedPreferences), read once into memory. */
class Settings(context: Context) {
    private val prefs = context.getSharedPreferences("settings", Context.MODE_PRIVATE)

    var autoplay: Boolean
        get() = prefs.getBoolean("autoplay", true)
        set(value) = prefs.edit().putBoolean("autoplay", value).apply()

    /** Seconds the end of a song and the start of the next overlap (0 = gapless). */
    var crossfade: Int
        get() = prefs.getInt("crossfade", 0)
        set(value) = prefs.edit().putInt("crossfade", value).apply()

    /** Turn loud masters down to YouTube's loudness target. */
    var normalize: Boolean
        get() = prefs.getBoolean("normalize", true)
        set(value) = prefs.edit().putBoolean("normalize", value).apply()

    /** Downloads also go to the phone's Music folder (Android 10+). */
    var exportDownloads: Boolean
        get() = prefs.getBoolean("exportDownloads", true)
        set(value) = prefs.edit().putBoolean("exportDownloads", value).apply()

    /** "", "en" or "uk" (Android 12 and older; 13+ keeps it per app in the system). */
    var language: String
        get() = prefs.getString("language", "") ?: ""
        set(value) = prefs.edit().putString("language", value).apply()
}
