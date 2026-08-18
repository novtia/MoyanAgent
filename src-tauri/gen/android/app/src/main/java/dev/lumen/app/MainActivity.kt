package dev.lumen.app

import android.os.Bundle
import android.view.View
import android.webkit.WebView
import androidx.activity.enableEdgeToEdge
import androidx.core.view.ViewCompat
import androidx.core.view.WindowInsetsAnimationCompat
import androidx.core.view.WindowInsetsCompat
import kotlin.math.max

class MainActivity : TauriActivity() {
  private var webView: WebView? = null

  override fun onCreate(savedInstanceState: Bundle?) {
    enableEdgeToEdge()
    super.onCreate(savedInstanceState)
    val content = findViewById<View>(android.R.id.content)
    ViewCompat.setOnApplyWindowInsetsListener(content) { _, insets ->
      applySafeInsets(insets)
      insets
    }
  }

  override fun onWebViewCreate(webView: WebView) {
    this.webView = webView
    val settings = webView.settings
    settings.useWideViewPort = true
    settings.loadWithOverviewMode = true
    settings.textZoom = 100
    settings.setSupportZoom(false)
    settings.builtInZoomControls = false
    settings.displayZoomControls = false

    ViewCompat.setOnApplyWindowInsetsListener(webView) { _, insets ->
      applySafeInsets(insets)
      insets
    }
    ViewCompat.setWindowInsetsAnimationCallback(
      webView,
      object : WindowInsetsAnimationCompat.Callback(
        WindowInsetsAnimationCompat.Callback.DISPATCH_MODE_CONTINUE_ON_SUBTREE,
      ) {
        override fun onProgress(
          insets: WindowInsetsCompat,
          runningAnimations: MutableList<WindowInsetsAnimationCompat>,
        ): WindowInsetsCompat {
          applySafeInsets(insets)
          return insets
        }
      },
    )
    ViewCompat.requestApplyInsets(webView)
  }

  private fun applySafeInsets(insets: WindowInsetsCompat) {
    val wv = webView ?: return
    val bars = insets.getInsets(
      WindowInsetsCompat.Type.systemBars() or WindowInsetsCompat.Type.displayCutout(),
    )
    val ime = insets.getInsets(WindowInsetsCompat.Type.ime())
    val d = resources.displayMetrics.density
    val top = bars.top / d
    val bottomNav = bars.bottom / d
    val imeBottom = max(0, ime.bottom) / d
    val left = bars.left / d
    val right = bars.right / d
    val imeFlag =
      if (imeBottom > 40.0) "r.dataset.ime='open';" else "delete r.dataset.ime;"
    val js =
      """
      (function(){
        var r=document.documentElement;
        r.style.setProperty('--safe-top','${top}px');
        r.style.setProperty('--safe-bottom','${bottomNav}px');
        r.style.setProperty('--ime-bottom','${imeBottom}px');
        r.style.setProperty('--safe-left','${left}px');
        r.style.setProperty('--safe-right','${right}px');
        $imeFlag
      })();
      """.trimIndent()
    wv.evaluateJavascript(js, null)
  }
}
