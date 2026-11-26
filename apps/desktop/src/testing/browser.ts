// Browsers deliver selectionchange asynchronously. The test DOM delivers it during Selection
// mutations, which re-enters an editor update while it is still rendering its new selection.
const deferred = new WeakSet<Event>();
let pending = false;
document.addEventListener(
  "selectionchange",
  (event) => {
    const focused = document.activeElement;
    if (deferred.has(event) || !(focused instanceof Element) || !focused.closest(".cm-editor")) {
      return;
    }
    event.stopImmediatePropagation();
    if (!pending) {
      pending = true;
      queueMicrotask(() => {
        pending = false;
        const notification = new Event("selectionchange");
        deferred.add(notification);
        document.dispatchEvent(notification);
      });
    }
  },
  true,
);
