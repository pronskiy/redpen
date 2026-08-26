// A2.3 replaces this with the streaming markdown renderer. For now it only proves
// the webview is alive inside a window that was created hidden.
const el = document.querySelector<HTMLElement>("#critique");
if (el) {
  el.dataset.state = "scaffold";
  console.log("[redpen] webview booted");
}
