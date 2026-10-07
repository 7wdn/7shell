<script setup lang="ts">
/**
 * The font picker: a field showing the chosen family, over a panel with a search box in it.
 *
 * The search is *inside* the panel rather than being the field itself, and that is the shape a list
 * of two hundred families asks for. The field's job is to say what is chosen — a name somebody reads
 * at a glance — and the panel's is to find another one; a field that turns into a search box on the
 * way in loses the first job and makes the second one start from a word nobody typed.
 *
 * The list is drawn here rather than by a select, because there is no select that searches: the
 * filter, the highlight and the keyboard belong to this component, and they are the ordinary ones —
 * arrows wrap, Enter takes what is highlighted, the top row is highlighted as the search narrows,
 * Escape closes. What is not ordinary is the row: every one is drawn in the face it offers, because
 * a name says very little about how a font reads.
 *
 * It reports what the list is pointing at as it moves, by pointer or by arrow key, and says when
 * nothing is any more: a caller whose choice is the thing being looked at — a font, a colour — shows
 * it before it is taken. Nothing here decides that; it is reported and no more.
 */
import { Check, ChevronDown, Search } from "@lucide/vue";
import { PopoverContent, PopoverPortal, PopoverRoot, PopoverTrigger } from "reka-ui";
import { computed, nextTick, ref } from "vue";

import { cn } from "@/lib/utils";

const props = withDefaults(
  defineProps<{
    /** The stored stack, which is what a choice is worth. */
    modelValue: string;
    /** Every stack on offer, in the order they are drawn. */
    options: readonly string[];
    /** What a stack is called on screen. */
    name: (value: string) => string;
    /** The control's name, for a reader who cannot see the label drawn beside it. */
    ariaLabel?: string;
    /** What the search box in the panel says while it is empty. */
    searchPlaceholder?: string;
    /** Whether a name typed into the search box may be taken as a family. */
    allowCustom?: boolean;
    /** How a typed name is turned into a stack — quoted, with a fallback after it. */
    normalizeCustom?: (value: string) => string;
    /** Shown in place of the list while the machine's families are still being read. */
    loading?: boolean;
    loadingText?: string;
    emptyText?: string;
    /** The line that offers a typed name, with the name in it. */
    useTypedText?: (value: string) => string;
    triggerClass?: string;
  }>(),
  {
    ariaLabel: undefined,
    searchPlaceholder: "",
    allowCustom: false,
    normalizeCustom: (value: string) => value,
    loading: false,
    loadingText: "",
    emptyText: "",
    useTypedText: (value: string) => value,
    triggerClass: "",
  },
);

const emit = defineEmits<{
  "update:modelValue": [value: string];
  /** The option the panel is on: what the window would look like wearing it. */
  preview: [value: string];
  /** Nothing is being pointed at any more: show what is actually chosen. */
  endPreview: [];
  /** The panel opened or closed, which is when the families are worth reading. */
  open: [value: boolean];
}>();

const open = ref(false);
/** What has been typed into the search box, which is nothing until somebody types. */
const searchText = ref("");
/** Which row the keyboard is on, or -1 while it is on none of them. */
const highlight = ref(-1);
const searchField = ref<HTMLInputElement | null>(null);
const rows = ref<HTMLElement[]>([]);

/** What the field shows: the chosen family's name. */
const shown = computed(() => props.name(props.modelValue));

/**
 * The rows left after what was typed.
 *
 * Searched by the name on screen rather than by the stack: somebody types 雅黑 or "source", never
 * `'Source Code Pro', monospace`, and a list matched against the stored text would be a list nobody
 * could search.
 */
const filtered = computed(() => {
  const needle = searchText.value.trim().toLowerCase();
  if (needle === "") {
    return props.options;
  }
  return props.options.filter((option) => props.name(option).toLowerCase().includes(needle));
});

/** The typed name, when it is worth a row of its own: something to take, and not already offered. */
const custom = computed(() => {
  const typed = searchText.value.trim();
  if (!props.allowCustom || typed === "") {
    return "";
  }
  const stack = props.normalizeCustom(typed);
  return props.options.includes(stack) ? "" : stack;
});

/** How many rows the keyboard can walk: the ones on screen, and the typed name if it is one. */
const rowCount = computed(() => filtered.value.length + (custom.value ? 1 : 0));

function onOpenChange(next: boolean): void {
  open.value = next;
  emit("open", next);
  if (!next) {
    searchText.value = "";
    highlight.value = -1;
    emit("endPreview");
    return;
  }
  void nextTick(() => {
    searchField.value?.focus();
    // The keyboard starts on the chosen family, so Enter on a list nobody has narrowed is a no-op
    // rather than a change to whatever happened to be first.
    const selected = filtered.value.indexOf(props.modelValue);
    highlight.value = selected >= 0 ? selected : 0;
    scrollHighlighted();
  });
}

function scrollHighlighted(): void {
  rows.value[highlight.value]?.scrollIntoView({ block: "nearest" });
}

/** What a row is worth when it is taken: the option itself, or the typed name as a stack. */
function rowValue(index: number): string {
  return index < filtered.value.length ? (filtered.value[index] ?? "") : custom.value;
}

function pick(index: number): void {
  const value = rowValue(index);
  if (value) {
    emit("update:modelValue", value);
  }
  onOpenChange(false);
}

/** Pointing at a row, which is what a preview is asked for. */
function preview(index: number): void {
  const value = rowValue(index);
  if (value) {
    emit("preview", value);
  }
}

function onSearch(): void {
  // The rows are repainted, so the highlight starts again at the top of what is left — which is the
  // row somebody is looking at after narrowing the list.
  highlight.value = 0;
  void nextTick(scrollHighlighted);
}

function onKeydown(event: KeyboardEvent): void {
  if (event.key === "ArrowDown" || event.key === "ArrowUp") {
    event.preventDefault();
    if (rowCount.value === 0) {
      return;
    }
    const step = event.key === "ArrowDown" ? 1 : -1;
    highlight.value = (highlight.value + step + rowCount.value) % rowCount.value;
    scrollHighlighted();
    preview(highlight.value);
    return;
  }
  if (event.key === "Enter") {
    event.preventDefault();
    if (highlight.value >= 0 && highlight.value < rowCount.value) {
      pick(highlight.value);
    }
    return;
  }
  if (event.key === "Escape") {
    onOpenChange(false);
  }
}
</script>

<template>
  <PopoverRoot :open="open" @update:open="onOpenChange">
    <PopoverTrigger as-child>
      <!--
        A button, because that is what it is: it says what is chosen and opens a panel. The name it
        carries for a reader who cannot see the label beside it is the row's own, and it says that it
        opens a list.
      -->
      <button
        type="button"
        :aria-label="props.ariaLabel"
        :title="props.ariaLabel"
        aria-haspopup="listbox"
        :aria-expanded="open"
        :class="
          cn(
            'flex items-center justify-between gap-2 rounded-md border border-border bg-background px-3 text-left text-sm text-foreground transition-colors',
            'hover:bg-accent/50 focus-visible:ring-1 focus-visible:ring-ring focus-visible:outline-none',
            props.triggerClass || 'h-9 w-full',
          )
        "
      >
        <span class="min-w-0 flex-1 truncate">{{ shown }}</span>
        <ChevronDown :size="14" class="shrink-0 text-muted-foreground" />
      </button>
    </PopoverTrigger>

    <PopoverPortal>
      <PopoverContent
        side="bottom"
        align="start"
        :side-offset="4"
        :collision-padding="8"
        class="z-50 w-[var(--reka-popover-trigger-width)] rounded-lg border border-border bg-card p-1 shadow-xl"
      >
        <!--
          The search box, at the top of the panel. A real search field, so the role comes from the
          markup — with the platform's own clear button turned off in the stylesheet, because this
          window draws that button itself and a second one in the same field would be one too many.
        -->
        <div class="relative mb-1">
          <Search
            :size="13"
            class="pointer-events-none absolute top-1/2 left-2 -translate-y-1/2 text-muted-foreground"
          />
          <input
            ref="searchField"
            v-model="searchText"
            type="search"
            role="combobox"
            aria-controls="font-select-rows"
            :aria-expanded="true"
            :aria-activedescendant="highlight >= 0 ? `font-select-row-${highlight}` : undefined"
            :placeholder="props.searchPlaceholder"
            :aria-label="props.searchPlaceholder"
            class="h-7 w-full rounded-md border border-border bg-background pr-2 pl-6 text-sm transition-colors select-text focus-visible:border-primary focus-visible:outline-none"
            @keydown="onKeydown"
            @input="onSearch"
          />
        </div>

        <div
          id="font-select-rows"
          role="listbox"
          :aria-label="props.ariaLabel"
          class="scroll-thin max-h-64 overflow-y-auto"
          @pointerleave="emit('endPreview')"
        >
          <p v-if="props.loading" class="px-2 py-6 text-center text-xs text-muted-foreground">
            {{ props.loadingText }}
          </p>

          <template v-else>
            <button
              v-for="(option, index) in filtered"
              :id="`font-select-row-${index}`"
              :key="option"
              ref="rows"
              type="button"
              role="option"
              :aria-selected="option === props.modelValue"
              :class="
                cn(
                  'flex w-full cursor-default items-center gap-2 rounded-md px-2 py-1.5 text-left text-sm outline-none select-none',
                  index === highlight ? 'bg-accent text-accent-foreground' : 'hover:bg-accent/60',
                )
              "
              :style="{ fontFamily: option }"
              @pointerenter="preview(index)"
              @click="pick(index)"
            >
              <Check :size="13" class="shrink-0" :class="option === props.modelValue ? '' : 'opacity-0'" />
              <span class="min-w-0 flex-1 truncate">{{ props.name(option) }}</span>
            </button>

            <!--
              The name somebody typed, offered as the last row: it is the one thing the list cannot
              contain, because it is not installed — a font on a network share, a name spelled
              differently, a family this machine's list does not carry.
            -->
            <button
              v-if="custom"
              :id="`font-select-row-${filtered.length}`"
              ref="rows"
              type="button"
              role="option"
              :aria-selected="false"
              :class="
                cn(
                  'flex w-full cursor-default items-center gap-2 rounded-md px-2 py-1.5 text-left text-sm outline-none select-none',
                  highlight === filtered.length ? 'bg-accent text-accent-foreground' : 'hover:bg-accent/60',
                )
              "
              @pointerenter="preview(filtered.length)"
              @click="pick(filtered.length)"
            >
              <Check :size="13" class="shrink-0 opacity-0" />
              <span class="min-w-0 flex-1 truncate">{{ props.useTypedText(searchText.trim()) }}</span>
            </button>

            <p v-if="filtered.length === 0 && !custom" class="px-2 py-6 text-center text-xs text-muted-foreground">
              {{ props.emptyText }}
            </p>
          </template>
        </div>
      </PopoverContent>
    </PopoverPortal>
  </PopoverRoot>
</template>
