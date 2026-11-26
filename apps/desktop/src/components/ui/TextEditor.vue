<script setup lang="ts">
import {
  history,
  historyKeymap,
  indentLess,
  insertNewline,
  insertNewlineKeepIndent,
  redo,
  standardKeymap,
} from "@codemirror/commands";
import { indentUnit } from "@codemirror/language";
import { Compartment, EditorState, Prec, Text, Transaction, type Extension } from "@codemirror/state";
import { drawSelection, EditorView, keymap } from "@codemirror/view";
import { computed, nextTick, onBeforeUnmount, onMounted, ref, useAttrs, useId, watch } from "vue";
import { useI18n } from "vue-i18n";

import { insertIndent, insertYamlNewline, lineSeparatorOf } from "@/lib/editor/commands";
import { matchesShortcut } from "@/lib/shortcuts/binding";
import { formatShortcutDisplay } from "@/lib/shortcuts/display";
import { resolveBindings } from "@/lib/shortcuts/registry";
import { useAppStore } from "@/stores/app";
import { usePreferencesStore } from "@/stores/preferences";
import { normalizeEditorIndentWidth } from "@/types/app";

defineOptions({ inheritAttrs: false });
const props = withDefaults(
  defineProps<{
    modelValue: string;
    label: string;
    id?: string;
    readonly?: boolean;
    busy?: boolean;
    yaml?: boolean;
    autofocus?: boolean;
  }>(),
  { readonly: false, busy: false, yaml: false, autofocus: true },
);
const emit = defineEmits<{
  "update:modelValue": [text: string];
  save: [];
  submit: [];
  cancel: [];
}>();

const attrs = useAttrs();
const { t } = useI18n();
const app = useAppStore();
const preferences = usePreferencesStore();
const host = ref<HTMLDivElement | null>(null);
const helpId = useId();
const fieldId = useId();
const tabFocusMode = ref(false);
const bindings = computed(() => resolveBindings(preferences.keybindings));
const width = computed(() => normalizeEditorIndentWidth(preferences.editorIndentWidth));
const spaces = computed(() => props.yaml || preferences.editorIndentStyle !== "tabs");
const readOnly = computed(() => props.readonly || props.busy);
const options = new Compartment();
let lineSeparator = "\n";
let view: EditorView | null = null;
let synchronizing = false;

const indentationLabel = computed(() =>
  t(spaces.value ? "common.editor.spaces" : "common.editor.tabs", { width: width.value }),
);
const focusBinding = computed(() => formatShortcutDisplay(bindings.value["editor.toggleTabFocus"], app.platform));
const help = computed(() => {
  const actions = [
    ["editor.save", "common.editor.saveKey"],
    ["editor.submit", "common.editor.submitKey"],
    ["editor.cancel", "common.editor.cancelKey"],
    ["editor.toggleTabFocus", "common.editor.focusKey"],
  ] as const;
  return [
    t("common.editor.indentKeys"),
    ...actions.flatMap(([action, label]) => {
      const binding = bindings.value[action];
      return binding === "" ? [] : [t(label, { key: formatShortcutDisplay(binding, app.platform) })];
    }),
  ].join(" · ");
});

function configuration(): Extension {
  return [
    EditorState.readOnly.of(readOnly.value),
    EditorView.editable.of(!readOnly.value),
    EditorState.tabSize.of(width.value),
    indentUnit.of(spaces.value ? " ".repeat(width.value) : "\t"),
    keymap.of([
      ...(!readOnly.value
        ? [{ key: "Tab", run: insertIndent, shift: indentLess, preventDefault: true, stopPropagation: true }]
        : []),
      {
        key: "Enter",
        run: (editor) => {
          if (!editor.state.readOnly) {
            if (preferences.editorAutoIndent && props.yaml) {
              return insertYamlNewline(editor);
            }
            return (preferences.editorAutoIndent ? insertNewlineKeepIndent : insertNewline)(editor);
          }
          return true;
        },
        shift: (editor) => !editor.state.readOnly && insertNewline(editor),
        preventDefault: true,
        stopPropagation: true,
      },
    ]),
    EditorView.contentAttributes.of({
      id: props.id ?? fieldId,
      "aria-label": props.label,
      "aria-describedby": helpId,
      "aria-readonly": String(readOnly.value),
      "aria-busy": String(props.busy),
      tabindex: "0",
      spellcheck: "false",
    }),
  ];
}

function toggleTabFocus(): void {
  tabFocusMode.value = !tabFocusMode.value;
  view?.setTabFocusMode(tabFocusMode.value);
}

function guardComposition(event: KeyboardEvent): void {
  // A handled CodeMirror key cancels its native default. Stop composition keys before that
  // handler instead, so the input method can accept candidates without inserting an editor line.
  if (event.isComposing || view?.compositionStarted) {
    event.stopPropagation();
  }
}

function onKey(event: KeyboardEvent): boolean {
  const actions = ["editor.save", "editor.submit", "editor.cancel", "editor.toggleTabFocus"] as const;
  for (const action of actions) {
    if (!matchesShortcut(event, bindings.value[action], app.platform)) {
      continue;
    }
    event.preventDefault();
    event.stopPropagation();
    if (event.repeat) {
      return true;
    }
    if (action === "editor.toggleTabFocus") {
      toggleTabFocus();
    } else if (action === "editor.cancel") {
      if (!props.busy) {
        emit("cancel");
      }
    } else if (!readOnly.value) {
      if (action === "editor.save") {
        emit("save");
      } else {
        emit("submit");
      }
    }
    return true;
  }
  return false;
}

const theme = EditorView.theme({
  "&": { height: "100%", fontSize: "inherit", color: "var(--foreground)", backgroundColor: "var(--background)" },
  "&.cm-focused": { outline: "none" },
  ".cm-scroller": { overflow: "auto", fontFamily: "var(--font-mono)", lineHeight: "inherit" },
  ".cm-content": { padding: "calc(var(--spacing) * 2) calc(var(--spacing) * 3)", caretColor: "var(--foreground)" },
  ".cm-line": { padding: "0" },
  ".cm-cursor": { borderLeftColor: "var(--foreground)" },
  "&.cm-focused .cm-selectionBackground, .cm-selectionBackground, ::selection": { backgroundColor: "var(--accent)" },
});

onMounted(() => {
  if (!host.value) {
    return;
  }
  lineSeparator = lineSeparatorOf(props.modelValue);
  view = new EditorView({
    parent: host.value,
    state: EditorState.create({
      doc: Text.of(props.modelValue.split(/\r\n|\r|\n/)),
      extensions: [
        options.of(configuration()),
        history(),
        drawSelection(),
        EditorView.lineWrapping,
        theme,
        Prec.highest(EditorView.domEventHandlers({ keydown: onKey })),
        keymap.of([{ key: "Mod-Shift-z", run: redo, preventDefault: true, stopPropagation: true }]),
        keymap.of([...historyKeymap, ...standardKeymap].map((binding) => ({ ...binding, stopPropagation: true }))),
        EditorView.updateListener.of((update) => {
          if (update.docChanged && !synchronizing) {
            emit("update:modelValue", update.state.doc.sliceString(0, update.state.doc.length, lineSeparator));
          }
        }),
      ],
    }),
  });
  if (props.autofocus) {
    view.focus();
  }
});

watch(
  () => props.modelValue,
  (text) => {
    if (!view || text === view.state.doc.sliceString(0, view.state.doc.length, lineSeparator)) {
      return;
    }
    lineSeparator = lineSeparatorOf(text);
    synchronizing = true;
    try {
      view.dispatch({
        changes: { from: 0, to: view.state.doc.length, insert: Text.of(text.split(/\r\n|\r|\n/)) },
        annotations: Transaction.addToHistory.of(false),
      });
    } finally {
      synchronizing = false;
    }
  },
);

watch(
  () => [width.value, spaces.value, readOnly.value, preferences.editorAutoIndent, props.label, props.id] as const,
  async (current, previous) => {
    view?.dispatch({ effects: options.reconfigure(configuration()) });
    if (props.autofocus && previous[2] && !current[2]) {
      await nextTick();
      view?.focus();
    }
  },
);

onBeforeUnmount(() => {
  view?.destroy();
  view = null;
});

defineExpose({ focus: () => view?.focus() });
</script>

<template>
  <div
    v-bind="attrs"
    class="flex min-h-0 flex-col overflow-hidden"
    data-text-editor
    @keydown.capture="guardComposition"
  >
    <div ref="host" class="min-h-0 flex-1 overflow-hidden" />
    <div
      class="flex shrink-0 flex-wrap items-center gap-x-3 gap-y-1 border-t border-border bg-muted/20 px-3 py-1 text-[11px] text-muted-foreground"
    >
      <span>{{ indentationLabel }}</span>
      <span v-if="props.yaml">{{ t("common.editor.yamlSpaces") }}</span>
      <button
        type="button"
        class="ml-auto rounded-sm px-1 py-0.5 transition-colors hover:bg-accent hover:text-foreground"
        :aria-pressed="readOnly || tabFocusMode"
        :disabled="readOnly"
        :title="focusBinding ? t('common.editor.toggleKey', { key: focusBinding }) : t('common.editor.toggle')"
        @click="toggleTabFocus"
      >
        {{ readOnly || tabFocusMode ? t("common.editor.tabFocus") : t("common.editor.tabIndent") }}
      </button>
    </div>
    <span :id="helpId" class="sr-only">{{ help }}</span>
  </div>
</template>
