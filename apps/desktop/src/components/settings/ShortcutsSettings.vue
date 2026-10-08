<script setup lang="ts">
/**
 * The keys this application reserves, and what somebody has made of them.
 *
 * A row per action: what it does, the key it answers to now, and the ways out — record another one,
 * put the default back, or unbind it. Recording is a state of the row rather than a dialog: the row
 * says "press a key", and the next press that names a key becomes the binding. That is also the only
 * way to record a combination the browser would otherwise act on itself — the press is taken before
 * anything else can have it.
 *
 * Two actions in one surface must not answer to the same key: whichever is listened for first wins,
 * silently and in an order nobody can see, and the loser is a key that "does not work". A row sharing
 * its key with another says which one.
 */
import { computed, ref } from "vue";
import { useI18n } from "vue-i18n";

import { eventToBinding, normalizeBinding } from "@/lib/shortcuts/binding";
import { formatShortcutDisplay } from "@/lib/shortcuts/display";
import {
  resolveBindings,
  SHORTCUT_DEFINITIONS,
  SHORTCUT_SCOPE_ORDER,
  type ShortcutDefinition,
} from "@/lib/shortcuts/registry";
import { useAppStore } from "@/stores/app";
import { usePreferencesStore } from "@/stores/preferences";

const { t } = useI18n();
const app = useAppStore();
const preferences = usePreferencesStore();

/** The action being recorded, or null while nothing is. */
const recording = ref<string | null>(null);

/** The bindings in force: the defaults, with whatever somebody has changed. */
const bindings = computed(() => resolveBindings(preferences.keybindings));

/** The surfaces that have actions, in the order this page lists them. */
const scopes = computed(() =>
  SHORTCUT_SCOPE_ORDER.map((scope) => ({
    scope,
    actions: SHORTCUT_DEFINITIONS.filter((definition) => definition.scope === scope),
  })).filter((group) => group.actions.length > 0),
);

/**
 * The other action in the same surface that answers to the same key, by action id.
 *
 * Compared through `normalizeBinding`, so `mod+v`, `Mod+V` and `V+Mod` are one key here — a file
 * somebody edited by hand is a file that can be spelled in more than one way, and a conflict that
 * only showed up for one spelling would be a conflict nobody sees.
 */
const conflicts = computed<Record<string, string>>(() => {
  const found: Record<string, string> = {};
  const byKey = new Map<string, string>();
  for (const definition of SHORTCUT_DEFINITIONS) {
    const binding = normalizeBinding(bindings.value[definition.id]);
    if (binding === null || binding === "") {
      continue;
    }
    const key = `${definition.scope}:${binding}`;
    const other = byKey.get(key);
    if (other === undefined) {
      byKey.set(key, definition.id);
      continue;
    }
    const label = SHORTCUT_DEFINITIONS.find((entry) => entry.id === other)?.labelKey;
    if (label !== undefined && found[definition.id] === undefined) {
      found[definition.id] = label;
    }
    if (found[other] === undefined) {
      found[other] = definition.labelKey;
    }
  }
  return found;
});

/** The key a row answers to, spelled the way this platform writes it — empty when it is unbound. */
function keyFor(definition: ShortcutDefinition): string {
  const binding = bindings.value[definition.id];
  return binding === "" ? "" : formatShortcutDisplay(binding, app.platform);
}

/** Whether somebody has touched this action: what decides if there is anything to put back. */
function changed(definition: ShortcutDefinition): boolean {
  return preferences.keybindings[definition.id] !== undefined;
}

function start(definition: ShortcutDefinition): void {
  recording.value = definition.id;
}

/** The press that becomes the binding, or the one that leaves the row as it was. */
function record(event: KeyboardEvent, definition: ShortcutDefinition): void {
  if (recording.value !== definition.id) {
    return;
  }

  // The press belongs to the recording, whatever it is: a combination the browser would act on is
  // exactly the kind somebody is recording, and letting it through would close the settings instead.
  event.preventDefault();
  event.stopPropagation();

  if (event.key === "Escape") {
    recording.value = null;
    return;
  }

  const binding = eventToBinding(event, app.platform);
  if (binding === null) {
    // A modifier on its own names no key: the row keeps waiting for one that does.
    return;
  }

  preferences.setKeybinding(definition.id, binding);
  recording.value = null;
}

function reset(definition: ShortcutDefinition): void {
  preferences.setKeybinding(definition.id, null);
  recording.value = null;
}

function unbind(definition: ShortcutDefinition): void {
  preferences.setKeybinding(definition.id, "");
  recording.value = null;
}
</script>

<template>
  <div class="space-y-5">
    <p class="text-xs text-muted-foreground">{{ t("settings.shortcuts.hint") }}</p>

    <section v-for="group in scopes" :key="group.scope" class="space-y-1.5">
      <h4 class="text-sm font-medium text-muted-foreground">{{ t(`settings.shortcuts.scope.${group.scope}`) }}</h4>

      <div
        v-for="definition in group.actions"
        :key="definition.id"
        class="flex flex-wrap items-center gap-2 rounded-md border border-border px-2 py-1.5"
      >
        <span class="min-w-0 flex-1 truncate text-xs">{{ t(definition.labelKey) }}</span>

        <span v-if="conflicts[definition.id]" class="truncate text-[11px] text-warning">
          {{ t("settings.shortcuts.conflict", { action: t(conflicts[definition.id] as string) }) }}
        </span>

        <kbd
          v-if="keyFor(definition) !== ''"
          class="shrink-0 rounded-sm border border-border bg-muted px-1.5 py-0.5 font-mono text-[11px]"
        >
          {{ keyFor(definition) }}
        </kbd>
        <span v-else class="shrink-0 text-[11px] text-muted-foreground">
          {{ t("settings.shortcuts.unbound") }}
        </span>

        <button
          type="button"
          class="shrink-0 rounded-md border px-2 py-0.5 text-[11px] transition-colors"
          :class="
            recording === definition.id
              ? 'border-primary text-primary'
              : 'border-border hover:bg-accent hover:text-foreground'
          "
          :aria-label="t('settings.shortcuts.changeFor', { action: t(definition.labelKey) })"
          @click="start(definition)"
          @keydown="record($event, definition)"
        >
          {{ recording === definition.id ? t("settings.shortcuts.press") : t("settings.shortcuts.change") }}
        </button>

        <button
          v-if="changed(definition)"
          type="button"
          class="shrink-0 rounded-md border border-border px-2 py-0.5 text-[11px] transition-colors hover:bg-accent hover:text-foreground"
          @click="reset(definition)"
        >
          {{ t("settings.shortcuts.reset") }}
        </button>

        <button
          v-if="changed(definition) && keyFor(definition) !== ''"
          type="button"
          class="shrink-0 rounded-md border border-border px-2 py-0.5 text-[11px] text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
          @click="unbind(definition)"
        >
          {{ t("settings.shortcuts.unbind") }}
        </button>
      </div>
    </section>
  </div>
</template>
