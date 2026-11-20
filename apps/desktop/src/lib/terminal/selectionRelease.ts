/**
 * xterm 6 keeps its document-wide drag listener until mouseup, even when a later move says the
 * primary button is already up. Recover a missed release before that listener extends the selection.
 */
interface SelectionTerminal {
  readonly modes: { readonly mouseTrackingMode: string };
  readonly options: { readonly macOptionClickForcesSelection?: boolean };
}

interface ReleaseOptions {
  isMac: boolean;
  released: () => void;
  cancelled: () => void;
}

export function guardSelectionRelease(
  surface: HTMLElement,
  terminal: SelectionTerminal,
  options: ReleaseOptions,
): { cancel: () => void; dispose: () => void } {
  const document = surface.ownerDocument;
  const view = document.defaultView ?? window;
  let selecting = false;
  let releaseTimer: number | undefined;

  function clearTimer(): void {
    if (releaseTimer !== undefined) {
      view.clearTimeout(releaseTimer);
      releaseTimer = undefined;
    }
  }

  function finish(cancelled: boolean): void {
    clearTimer();
    if (!selecting) {
      return;
    }
    selecting = false;
    // There is no public stop-drag API. xterm's selection listener accepts any mouseup, whereas
    // its PTY mouse reporter rejects button >= 3. This ends the drag without clearing rectangular
    // selections or injecting a left-button release into another terminal's remote application.
    document.dispatchEvent(new MouseEvent("mouseup", { bubbles: true, button: 3, buttons: 0 }));
    if (cancelled) {
      options.cancelled();
    } else {
      options.released();
    }
  }

  const cancel = (): void => finish(true);

  function onDown(event: MouseEvent): void {
    if (event.button !== 0) {
      return;
    }
    const forced = options.isMac
      ? event.altKey && terminal.options.macOptionClickForcesSelection === true
      : event.shiftKey;
    selecting = terminal.modes.mouseTrackingMode === "none" || forced;
  }

  function onMove(event: MouseEvent): void {
    if (selecting && typeof event.buttons === "number" && (event.buttons & 1) === 0) {
      finish(false);
    }
  }

  function onUp(event: MouseEvent): void {
    if (event.button === 0 && selecting) {
      selecting = false;
      clearTimer();
      options.released();
    }
  }

  function onPointerUp(event: PointerEvent): void {
    if (event.button === 0 && selecting) {
      // Let the compatibility mouseup reach xterm first, including its normal Alt-click handling.
      clearTimer();
      releaseTimer = view.setTimeout(() => finish(false), 0);
    }
  }

  function onVisibilityChange(): void {
    if (document.visibilityState === "hidden") {
      cancel();
    }
  }

  // A new press elsewhere also ends an abandoned drag from this pane. Capture must run before
  // xterm's document mousemove, which otherwise changes the selection and stops immediate propagation.
  view.addEventListener("mousedown", cancel, true);
  surface.addEventListener("mousedown", onDown, true);
  document.addEventListener("mousemove", onMove, true);
  view.addEventListener("mouseup", onUp);
  view.addEventListener("pointerup", onPointerUp, true);
  view.addEventListener("pointercancel", cancel, true);
  view.addEventListener("blur", cancel);
  document.addEventListener("visibilitychange", onVisibilityChange);

  return {
    cancel,
    dispose: () => {
      cancel();
      view.removeEventListener("mousedown", cancel, true);
      surface.removeEventListener("mousedown", onDown, true);
      document.removeEventListener("mousemove", onMove, true);
      view.removeEventListener("mouseup", onUp);
      view.removeEventListener("pointerup", onPointerUp, true);
      view.removeEventListener("pointercancel", cancel, true);
      view.removeEventListener("blur", cancel);
      document.removeEventListener("visibilitychange", onVisibilityChange);
    },
  };
}
