<script setup lang="ts">
/**
 * A release's notes, drawn.
 *
 * The blocks come from `lib/updates/notes`, which reads the subset of markdown a changelog body is
 * written in. What this adds is the drawing — and the decision that nothing else draws them: the
 * same notes appear where a release is described, so there is one reading of them rather than two
 * that could disagree about what a line means.
 */
import { computed } from "vue";

import { parseNotes } from "@/lib/updates/notes";

const props = defineProps<{ markdown: string }>();

const blocks = computed(() => parseNotes(props.markdown));
</script>

<template>
  <div class="space-y-0.5 text-xs leading-relaxed">
    <template v-for="(block, index) in blocks" :key="index">
      <p
        v-if="block.kind === 'heading'"
        class="mt-2 mb-1 font-semibold first:mt-0"
        :class="block.level <= 2 ? 'text-sm' : 'text-xs'"
      >
        <template v-for="(span, spanIndex) in block.spans" :key="spanIndex">
          <strong v-if="span.kind === 'strong'">{{ span.text }}</strong>
          <code v-else-if="span.kind === 'code'" class="rounded bg-muted px-1 py-0.5 text-[11px]">{{ span.text }}</code>
          <span v-else-if="span.kind === 'link'" class="text-primary underline">{{ span.text }}</span>
          <span v-else>{{ span.text }}</span>
        </template>
      </p>

      <ul v-else-if="block.kind === 'bullets' && !block.ordered" class="my-1 list-disc space-y-0.5 pl-4">
        <li v-for="(item, itemIndex) in block.items" :key="itemIndex">
          <template v-for="(span, spanIndex) in item" :key="spanIndex">
            <strong v-if="span.kind === 'strong'">{{ span.text }}</strong>
            <code v-else-if="span.kind === 'code'" class="rounded bg-muted px-1 py-0.5 text-[11px]">{{
              span.text
            }}</code>
            <span v-else-if="span.kind === 'link'" class="text-primary underline">{{ span.text }}</span>
            <span v-else>{{ span.text }}</span>
          </template>
        </li>
      </ul>

      <ol v-else-if="block.kind === 'bullets'" class="my-1 list-decimal space-y-0.5 pl-4">
        <li v-for="(item, itemIndex) in block.items" :key="itemIndex">
          <template v-for="(span, spanIndex) in item" :key="spanIndex">
            <strong v-if="span.kind === 'strong'">{{ span.text }}</strong>
            <code v-else-if="span.kind === 'code'" class="rounded bg-muted px-1 py-0.5 text-[11px]">{{
              span.text
            }}</code>
            <span v-else-if="span.kind === 'link'" class="text-primary underline">{{ span.text }}</span>
            <span v-else>{{ span.text }}</span>
          </template>
        </li>
      </ol>

      <p v-else class="my-1 first:mt-0">
        <template v-for="(span, spanIndex) in block.spans" :key="spanIndex">
          <strong v-if="span.kind === 'strong'">{{ span.text }}</strong>
          <code v-else-if="span.kind === 'code'" class="rounded bg-muted px-1 py-0.5 text-[11px]">{{ span.text }}</code>
          <span v-else-if="span.kind === 'link'" class="text-primary underline">{{ span.text }}</span>
          <span v-else>{{ span.text }}</span>
        </template>
      </p>
    </template>
  </div>
</template>
