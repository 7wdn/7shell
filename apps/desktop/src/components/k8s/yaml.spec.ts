/**
 * The YAML view: what the printer writes, and what it refuses to write.
 *
 * No parser reads this back — the view is read-only — so the assertions are about the two things a
 * reader depends on: that the text is shaped like the document (a list item's keys under it, a
 * nested object indented), and that a Secret's values are not in it while its keys are.
 */
import { describe, expect, it } from "vitest";

import { redactedForDisplay, toYaml } from "@/components/k8s/yaml";

describe("toYaml", () => {
  it("writes an object the way a manifest is written", () => {
    const text = toYaml({
      apiVersion: "v1",
      kind: "Pod",
      metadata: { name: "web-0", labels: { app: "web" } },
      spec: { containers: [{ name: "main", image: "nginx:1.27" }] },
    });

    expect(text).toBe(
      [
        "apiVersion: v1",
        "kind: Pod",
        "metadata:",
        "  name: web-0",
        "  labels:",
        "    app: web",
        "spec:",
        "  containers:",
        "    - name: main",
        "      image: nginx:1.27",
        "",
      ].join("\n"),
    );
  });

  it("quotes a string that would otherwise read as something else", () => {
    const text = toYaml({ a: "1.0", b: "true", c: "with: colon", d: "", e: " padded" });
    expect(text).toContain('a: "1.0"');
    expect(text).toContain('b: "true"');
    expect(text).toContain('c: "with: colon"');
    expect(text).toContain('d: ""');
    expect(text).toContain('e: " padded"');
  });

  it("writes empty collections inline rather than as a key with nothing under it", () => {
    expect(toYaml({ empty: {}, list: [], nothing: null })).toBe("empty: {}\nlist: []\nnothing: null\n");
  });

  it("writes booleans and numbers as themselves", () => {
    expect(toYaml({ replicas: 3, ready: true })).toBe("replicas: 3\nready: true\n");
  });
});

describe("redactedForDisplay", () => {
  it("keeps a Secret's keys and removes its values", () => {
    const secret = {
      apiVersion: "v1",
      kind: "Secret",
      metadata: { name: "tls" },
      data: { "tls.crt": "LS0tLS1CRUdJTg==", "tls.key": "c2VjcmV0" },
      stringData: { password: "hunter2" },
    };

    const shown = redactedForDisplay(secret);
    expect(shown.data).toEqual({ "tls.crt": "***", "tls.key": "***" });
    expect(shown.stringData).toEqual({ password: "***" });
    expect(JSON.stringify(shown)).not.toContain("hunter2");
    expect(JSON.stringify(shown)).not.toContain("LS0tLS1CRUdJTg");
    // The object the server sent is left alone: this is a view, not a rewrite.
    expect(secret.data["tls.key"]).toBe("c2VjcmV0");
  });

  it("leaves every other kind exactly as it was", () => {
    const configMap = { kind: "ConfigMap", data: { key: "value" } };
    expect(redactedForDisplay(configMap)).toEqual(configMap);
  });
});
