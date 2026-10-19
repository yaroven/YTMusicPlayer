# Pages call into these from JavaScript.
-keepclassmembers class * {
    @android.webkit.JavascriptInterface <methods>;
}
