/** xterm's onData also carries protocol replies. Only keyboard, text entry and paste are broadcast. */
export class TerminalInputOrigin {
  private expected = new Set<string>();
  private textPending = false;
  private textTimer: ReturnType<typeof setTimeout> | undefined;

  key(data: string): void {
    this.expected.add(data);
    queueMicrotask(() => this.expected.delete(data));
  }

  paste(text: string): void {
    const prepared = text.replace(/\r?\n/g, "\r");
    this.key(prepared);
    this.key(`\x1b[200~${prepared}\x1b[201~`);
  }

  text(): void {
    this.textPending = true;
    clearTimeout(this.textTimer);
    // Composition commits after the native event. Keep one more turn for xterm's own deferred
    // textarea read; control sequences still cannot be mistaken for composed text.
    this.textTimer = setTimeout(() => {
      this.textTimer = setTimeout(() => {
        this.textPending = false;
      }, 0);
    }, 0);
  }

  take(data: string): boolean {
    if (this.expected.delete(data)) {
      return true;
    }
    const printable = [...data].every((character) => character.charCodeAt(0) >= 32 && character.charCodeAt(0) !== 127);
    if (this.textPending && data.length > 0 && printable) {
      this.textPending = false;
      return true;
    }
    return false;
  }

  dispose(): void {
    clearTimeout(this.textTimer);
    this.expected.clear();
    this.textPending = false;
  }
}
