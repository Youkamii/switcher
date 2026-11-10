// Isolated WebKit renderer check. No Tauri backend or real account store is loaded.
import AppKit
import WebKit

final class Receiver: NSObject, WKScriptMessageHandler {
    weak var webview: WKWebView?
    func userContentController(_ controller: WKUserContentController, didReceive message: WKScriptMessage) {
        if message.name == "tick", let id = message.body as? Int {
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.03) { self.webview?.evaluateJavaScript("window.__uiTick(\(id))", completionHandler: nil) }
            return
        }
        guard let result = message.body as? String else { exit(2) }
        print(result)
        exit(0)
    }
}
let app = NSApplication.shared
app.setActivationPolicy(.accessory)
let configuration = WKWebViewConfiguration()
configuration.websiteDataStore = .nonPersistent()
configuration.preferences.setValue(true, forKey: "allowFileAccessFromFileURLs")
let receiver = Receiver()
configuration.userContentController.add(receiver, name: "report")
configuration.userContentController.add(receiver, name: "tick")
let width = Double(CommandLine.arguments[2])!
let panel = NSPanel(contentRect: NSRect(x: -10000, y: -10000, width: width, height: 1100), styleMask: [.borderless, .nonactivatingPanel], backing: .buffered, defer: false)
let webview = WKWebView(frame: panel.contentView!.bounds, configuration: configuration)
receiver.webview = webview
webview.autoresizingMask = [.width, .height]
panel.contentView!.addSubview(webview)
panel.orderFrontRegardless()
let entry = URL(fileURLWithPath: CommandLine.arguments[1])
webview.loadFileURL(entry, allowingReadAccessTo: entry.deletingLastPathComponent())
DispatchQueue.main.asyncAfter(deadline: .now() + 30) {
    webview.evaluateJavaScript("JSON.stringify({debug:window.__uiDebug,ready:document.readyState,body:document.body.innerText.slice(0,300),scripts:[...document.scripts].map(s=>s.src)})") { value, error in
        print("WebKit diagnostic: \(value ?? String(describing:error))"); exit(2)
    }
}
app.run()
