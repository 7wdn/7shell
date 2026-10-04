<script setup lang="ts">
/**
 * The dropdown, in the one shape the whole interface uses.
 *
 * The platform's own select looks like the platform it was drawn on rather than like the window it
 * is drawn in: its list is a separate OS surface that no stylesheet reaches, it cannot show a
 * second line per row, and on Windows it renders in the system colours whatever this application is
 * wearing. The list here is drawn by this application instead, from the same tokens as everything
 * else — which is the point of having a component rather than four copies of a platform control.
 *
 * What that costs is everything the platform used to do for free, so it is all here, once:
 *
 * 1. **The list is drawn beside the trigger, not inside the document flow.** It is teleported to
 *    the body and positioned by reka-ui's popper, which flips it above when there is no room below
 *    and keeps it inside the window — a dialog's own scroll box would otherwise clip it.
 * 2. **The keyboard works the way a select's does.** Opening with arrow keys, moving with arrows,
 *    typeahead on the labels, Home and End, Escape to close without choosing, and focus back on
 *    the trigger afterwards.
 * 3. **The empty string is a choice, not an absence.** "No group" and "the busiest interface" are
 *    real answers whose value is `""`. reka-ui refuses the empty string as an item value — it
 *    throws, because an empty model value is how it spells "nothing is chosen" — so both sides are
 *    translated through a private token and `""` behaves like any other value. The placeholder
 *    really does mean "nothing chosen yet".
 *
 * No text is written here: the labels, the placeholder and the accessible name all arrive from the
 * caller, which is where the translation is.
 */
import { Check, ChevronDown, ChevronUp } from "@lucide/vue";
import {
  SelectContent,
  SelectItem,
  SelectItemIndicator,
  SelectItemText,
  SelectPortal,
  SelectRoot,
  SelectScrollDownButton,
  SelectScrollUpButton,
  SelectTrigger,
  SelectViewport,
} from "reka-ui";
import { computed, useAttrs } from "vue";

import { cn } from "@/lib/utils";

export interface SelectOption {
  /** What the choice is worth. `""` is allowed and is a value like any other. */
  value: string;
  /** The line the reader chooses by, and the text typeahead matches. */
  label: string;
  /**
   * What tells this choice apart from one with the same name, in the monospace face after an em
   * dash: an endpoint, a path, a name on the wire. It joins the option's accessible name too,
   * because two sessions called `lab` are told apart by where they lead and by nothing else.
   */
  detail?: string;
  disabled?: boolean;
}

const props = withDefaults(
  defineProps<{
    modelValue: string;
    options: readonly SelectOption[];
    /** Shown when the value is none of the options — a control with nothing chosen yet. */
    placeholder?: string;
    /** `compact` is the density of a panel header; `default` the density of a form field. */
    size?: "default" | "compact";
    disabled?: boolean;
    /** The control's name, for a reader who cannot see the label drawn beside it. */
    ariaLabel?: string;
    id?: string;
    /** Escape hatches for a caller whose control is not the plain one — a wider list, a mono face. */
    triggerClass?: string;
    contentClass?: string;
  }>(),
  {
    placeholder: "",
    size: "default",
    disabled: false,
    ariaLabel: undefined,
    id: undefined,
    triggerClass: "",
    contentClass: "",
  },
);

const emit = defineEmits<{ "update:modelValue": [value: string] }>();

/**
 * What a caller puts on the component goes on the trigger, because the trigger is the control.
 *
 * It cannot be left to the default attribute fall-through: this component draws two roots — the
 * trigger and the list it teleports to the body — and a fragment has nowhere for a stray class or
 * `data-` attribute to land.
 */
defineOptions({ inheritAttrs: false });
const attrs = useAttrs();
const passthrough = computed(() => {
  const { class: _class, ...rest } = attrs;
  return rest;
});

/** The private stand-in for `""`, which reka-ui reads as "nothing is chosen" and will not match. */
const NONE = "\u0000sevenshell-none";
const inner = (value: string): string => (value === "" ? NONE : value);
const outer = (value: string): string => (value === NONE ? "" : value);

/** What the trigger says: the chosen option's label, or the prompt when there is no such option. */
const chosen = computed(() => props.options.find((option) => option.value === props.modelValue));

const compact = computed(() => props.size === "compact");

const triggerClass = computed(() =>
  cn(
    "inline-flex select-none items-center justify-between gap-2 rounded-md border border-border bg-background text-left text-foreground transition-colors",
    "hover:bg-accent/50 focus-visible:ring-1 focus-visible:ring-ring focus-visible:outline-none",
    "disabled:cursor-not-allowed disabled:opacity-60",
    compact.value ? "px-1.5 py-0.5 text-[11px]" : "w-full px-3 py-2 text-sm",
    props.triggerClass,
    attrs.class as string | undefined,
  ),
);

const contentClass = computed(() =>
  cn(
    "z-50 max-h-[min(20rem,var(--reka-select-content-available-height))] overflow-hidden rounded-lg border border-border bg-card shadow-xl",
    "min-w-[var(--reka-select-trigger-width)]",
    props.contentClass,
  ),
);

const itemClass = computed(() =>
  cn(
    "relative flex w-full cursor-default items-center gap-2 rounded-md outline-none select-none",
    "data-[highlighted]:bg-accent data-[highlighted]:text-accent-foreground",
    "data-[disabled]:pointer-events-none data-[disabled]:opacity-50",
    compact.value ? "py-1 pr-1.5 pl-5 text-[11px]" : "py-1.5 pr-2 pl-6 text-sm",
  ),
);

/** Where the tick sits, and how big it is: the column the item's own padding reserves for it. */
const checkClass = computed(() =>
  compact.value
    ? "absolute left-1.5 flex items-center justify-center"
    : "absolute left-2 flex items-center justify-center",
);

const scrollButtonClass = "flex cursor-default items-center justify-center py-1 text-muted-foreground";
</script>

<template>
  <SelectRoot
    :model-value="inner(props.modelValue)"
    :disabled="props.disabled"
    @update:model-value="(value: string) => emit('update:modelValue', outer(value))"
  >
    <SelectTrigger
      v-bind="passthrough"
      :id="props.id"
      :class="triggerClass"
      :aria-label="props.ariaLabel"
      :title="props.ariaLabel"
      :disabled="props.disabled"
    >
      <span class="min-w-0 flex-1 truncate" :class="chosen ? '' : 'text-muted-foreground'">
        {{ chosen?.label ?? props.placeholder }}
        <span v-if="chosen?.detail" class="text-muted-foreground">— {{ chosen.detail }}</span>
      </span>
      <ChevronDown :size="compact ? 11 : 14" class="shrink-0 text-muted-foreground" />
    </SelectTrigger>

    <SelectPortal>
      <SelectContent position="popper" :side-offset="4" :class="contentClass">
        <SelectScrollUpButton :class="scrollButtonClass">
          <ChevronUp :size="14" />
        </SelectScrollUpButton>

        <SelectViewport class="p-1">
          <SelectItem
            v-for="option in props.options"
            :key="option.value"
            :value="inner(option.value)"
            :text-value="option.label"
            :disabled="option.disabled"
            :class="itemClass"
          >
            <SelectItemIndicator :class="checkClass">
              <Check :size="compact ? 11 : 13" />
            </SelectItemIndicator>
            <SelectItemText class="min-w-0 flex-1 truncate">
              {{ option.label }}
              <span v-if="option.detail" class="font-mono text-[11px] text-muted-foreground">
                — {{ option.detail }}
              </span>
            </SelectItemText>
          </SelectItem>
        </SelectViewport>

        <SelectScrollDownButton :class="scrollButtonClass">
          <ChevronDown :size="14" />
        </SelectScrollDownButton>
      </SelectContent>
    </SelectPortal>
  </SelectRoot>
</template>
