package sk.newlevel.presenterstage

import android.annotation.SuppressLint
import android.content.Intent
import android.os.Build
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.view.View
import android.view.WindowManager
import android.webkit.PermissionRequest
import android.webkit.WebChromeClient
import android.webkit.WebResourceError
import android.webkit.WebResourceRequest
import android.webkit.WebSettings
import android.webkit.WebView
import android.webkit.WebViewClient

/**
 * Presenter Stage — a deliberately tiny, full-screen WebView kiosk for church
 * stage displays. It exists so the stage runs on ANY Android TV without a
 * third-party kiosk browser (Fully Kiosk) and without depending on a per-brand
 * browser (e.g. com.tcl.browser, absent on Sharp/MediaTek TVs).
 *
 * The server watchdog (presenter-server `android_stage.rs`) installs this APK
 * via ADB and opens it with:
 *   am start -a android.intent.action.VIEW -d <stage-url> sk.newlevel.presenterstage
 * The activity reads the URL from the VIEW intent and shows it full-screen,
 * with autoplay + WebRTC (WHEP/NDI video) enabled and the screen kept on.
 */
class StageActivity : android.app.Activity() {

    private lateinit var webView: WebView
    private val handler = Handler(Looper.getMainLooper())
    private var currentUrl: String? = null

    @SuppressLint("SetJavaScriptEnabled")
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        window.addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)

        webView = WebView(this)
        setContentView(webView)

        // Allow `chrome://inspect` debugging of the stage from a dev machine.
        WebView.setWebContentsDebuggingEnabled(true)

        webView.settings.apply {
            javaScriptEnabled = true
            domStorageEnabled = true
            // Stage video (NDI WHEP / HTML5) must autoplay without a user gesture.
            mediaPlaybackRequiresUserGesture = false
            loadWithOverviewMode = true
            useWideViewPort = true
            cacheMode = WebSettings.LOAD_DEFAULT
            // The stage is served over http on the LAN.
            mixedContentMode = WebSettings.MIXED_CONTENT_ALWAYS_ALLOW
        }

        webView.webChromeClient = object : WebChromeClient() {
            override fun onPermissionRequest(request: PermissionRequest) {
                // The stage uses WebRTC (WHEP) to receive NDI video; grant the
                // web resources it asks for so playback is never blocked.
                runOnUiThread { request.grant(request.resources) }
            }
        }

        webView.webViewClient = object : WebViewClient() {
            override fun onReceivedError(
                view: WebView,
                request: WebResourceRequest,
                error: WebResourceError,
            ) {
                // Retry only the main stage page (not every failed subresource),
                // so a transient server/network blip self-heals.
                if (request.isForMainFrame) scheduleReload()
            }
        }

        loadFromIntent(intent)
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        // singleTask: a relaunch (watchdog re-firing the VIEW intent, possibly
        // with a new URL) reuses this instance — load the new URL.
        setIntent(intent)
        loadFromIntent(intent)
    }

    private fun loadFromIntent(intent: Intent?) {
        val url = intent?.dataString?.takeIf { it.isNotBlank() }
        when {
            url != null -> {
                currentUrl = url
                webView.loadUrl(url)
            }
            // Opened from the launcher with no URL (and none loaded yet): show a
            // placeholder rather than a blank screen until the server launches us.
            currentUrl == null -> webView.loadData(
                "<html><body style=\"margin:0;background:#0f172a;color:#94a3b8;" +
                    "font-family:sans-serif;display:flex;align-items:center;" +
                    "justify-content:center;height:100vh\">" +
                    "<h2>Presenter Stage — waiting for server…</h2></body></html>",
                "text/html",
                "utf-8",
            )
        }
    }

    private fun scheduleReload() {
        handler.removeCallbacksAndMessages(null)
        handler.postDelayed({ currentUrl?.let { webView.loadUrl(it) } }, RELOAD_DELAY_MS)
    }

    override fun onResume() {
        super.onResume()
        enterImmersive()
        webView.onResume()
    }

    override fun onPause() {
        webView.onPause()
        super.onPause()
    }

    override fun onWindowFocusChanged(hasFocus: Boolean) {
        super.onWindowFocusChanged(hasFocus)
        if (hasFocus) enterImmersive()
    }

    @Suppress("DEPRECATION")
    private fun enterImmersive() {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
            window.setDecorFitsSystemWindows(false)
        }
        window.decorView.systemUiVisibility = (
            View.SYSTEM_UI_FLAG_IMMERSIVE_STICKY
                or View.SYSTEM_UI_FLAG_FULLSCREEN
                or View.SYSTEM_UI_FLAG_HIDE_NAVIGATION
                or View.SYSTEM_UI_FLAG_LAYOUT_STABLE
                or View.SYSTEM_UI_FLAG_LAYOUT_FULLSCREEN
                or View.SYSTEM_UI_FLAG_LAYOUT_HIDE_NAVIGATION
            )
    }

    override fun onDestroy() {
        handler.removeCallbacksAndMessages(null)
        webView.destroy()
        super.onDestroy()
    }

    private companion object {
        const val RELOAD_DELAY_MS = 3000L
    }
}
