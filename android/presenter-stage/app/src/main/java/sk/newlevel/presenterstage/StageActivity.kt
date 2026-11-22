package sk.newlevel.presenterstage

import android.annotation.SuppressLint
import android.content.Context
import android.content.Intent
import android.net.wifi.WifiManager
import android.os.Build
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.os.SystemClock
import android.util.Log
import android.view.View
import android.view.WindowManager
import android.webkit.PermissionRequest
import android.webkit.RenderProcessGoneDetail
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
    private var wifiLock: WifiManager.WifiLock? = null
    private var resumed = false
    private var rendererDeaths = 0
    private var lastRendererDeathAt = 0L

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        window.addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)

        webView = createWebView()
        setContentView(webView)

        // Allow `chrome://inspect` debugging of the stage from a dev machine.
        WebView.setWebContentsDebuggingEnabled(true)

        wifiLock = createWifiLock()
        loadFromIntent(intent)
    }

    /** A configured stage WebView (also used to replace one whose renderer died). */
    @SuppressLint("SetJavaScriptEnabled")
    private fun createWebView(): WebView {
        val stage = WebView(this)
        stage.settings.apply {
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

        stage.webChromeClient = object : WebChromeClient() {
            override fun onPermissionRequest(request: PermissionRequest) {
                // The stage uses WebRTC (WHEP) to receive NDI video; grant the
                // web resources it asks for so playback is never blocked.
                runOnUiThread { request.grant(request.resources) }
            }
        }

        stage.webViewClient = object : WebViewClient() {
            override fun onReceivedError(
                view: WebView,
                request: WebResourceRequest,
                error: WebResourceError,
            ) {
                // Retry only the main stage page (not every failed subresource),
                // so a transient server/network blip self-heals.
                if (request.isForMainFrame) scheduleReload()
            }

            // #990: a killed or crashed renderer (a 935 MB TV under memory
            // pressure) leaves this WebView dead for good — without this the
            // app stays foreground on a dark page. Replace it and reload.
            override fun onRenderProcessGone(
                view: WebView,
                detail: RenderProcessGoneDetail,
            ): Boolean {
                val crashed = Build.VERSION.SDK_INT >= Build.VERSION_CODES.O && detail.didCrash()
                val delay = rendererRestartDelay()
                Log.w(
                    TAG,
                    "stage WebView renderer gone (crashed=$crashed, death $rendererDeaths) — " +
                        "recreating it, reload in $delay ms (#990)",
                )
                recreateWebView(delay)
                return true
            }
        }
        return stage
    }

    /**
     * A stage on Wi-Fi must not sleep between packets: power save adds up to
     * ~100 ms to a text change (#879, SD3). The lock does nothing on a wired TV.
     */
    @Suppress("DEPRECATION")
    private fun createWifiLock(): WifiManager.WifiLock? {
        val wifi = applicationContext.getSystemService(Context.WIFI_SERVICE) as? WifiManager
        val mode = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
            WifiManager.WIFI_MODE_FULL_LOW_LATENCY
        } else {
            WifiManager.WIFI_MODE_FULL_HIGH_PERF
        }
        return wifi?.createWifiLock(mode, "presenter-stage")?.apply { setReferenceCounted(false) }
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        // singleTask: a relaunch (watchdog re-firing the VIEW intent, possibly
        // with a new URL) reuses this instance — load the new URL.
        setIntent(intent)
        // This load supersedes a pending delayed reload (#990), never twice.
        handler.removeCallbacksAndMessages(null)
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

    /**
     * #990: the wait before reloading after a renderer death — 3 s, doubling
     * per death up to ~96 s, so a page that itself exhausts memory is not a
     * tight crash → recreate → crash loop. A renderer that lived 5 min resets
     * the count.
     */
    private fun rendererRestartDelay(): Long {
        val now = SystemClock.elapsedRealtime()
        if (now - lastRendererDeathAt > RENDERER_STABLE_MS) rendererDeaths = 0
        lastRendererDeathAt = now
        rendererDeaths += 1
        return RELOAD_DELAY_MS shl (rendererDeaths - 1).coerceAtMost(5)
    }

    /**
     * #990: drop the dead WebView, put a fresh one in its place, and load the
     * stage URL again after [delayMs] (the placeholder if the server never
     * launched us). A paused activity leaves the new WebView paused too.
     */
    private fun recreateWebView(delayMs: Long) {
        handler.removeCallbacksAndMessages(null)
        val dead = webView
        webView = createWebView()
        setContentView(webView)
        dead.destroy()
        if (!resumed) webView.onPause()
        handler.postDelayed({
            val url = currentUrl
            if (url != null) webView.loadUrl(url) else loadFromIntent(intent)
        }, delayMs)
    }

    private fun scheduleReload() {
        handler.removeCallbacksAndMessages(null)
        handler.postDelayed({ currentUrl?.let { webView.loadUrl(it) } }, RELOAD_DELAY_MS)
    }

    override fun onResume() {
        super.onResume()
        resumed = true
        enterImmersive()
        webView.onResume()
        wifiLock?.acquire()
    }

    override fun onPause() {
        resumed = false
        wifiLock?.takeIf { it.isHeld }?.release()
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
        const val TAG = "PresenterStage"
        const val RENDERER_STABLE_MS = 5 * 60 * 1000L
    }
}
