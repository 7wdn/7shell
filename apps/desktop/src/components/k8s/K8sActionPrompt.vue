<script setup lang="ts">
/**
 * The question before a write: what is about to happen, to what, and with the one extra answer some
 * actions need.
 *
 * A dialog rather than a strip inside the row, because these are the actions that cannot be taken
 * back — a deleted StatefulSet takes its volumes' claims with it — and a question has to be readable
 * on its own. The title names the action, the message names the object and what happens to it, and
 * the button repeats the verb; "OK" is never a label for a delete.
 *
 * The two extra answers are here rather than in a dialog of their own: a force delete is a checkbox
 * under the same question, and a scale is a number in it. Both belong to the confirmation, not after
 * it, because answering "yes" and then "how many" is two moments to change one's mind in the middle
 * of a single intent.
 */
import { computed, ref, watch } from "vue";
import { useI18n } from "vue-i18n";

import ConfirmDialog from "@/components/ui/ConfirmDialog.vue";
import { nameOf, namespaceOf, numberAt, type K8sObject } from "@/components/k8s/objects";
import type { ResourceDescriptor } from "@/components/k8s/resources";

const props = defineProps<{
  open: boolean;
  kind: "delete" | "restart" | "scale" | "cordon" | "uncordon" | "drain" | null;
  descriptor: ResourceDescriptor | null;
  object: K8sObject | null;
  busy: boolean;
}>();

const emit = defineEmits<{
  confirm: [answer: { readonly force: boolean; readonly replicas: number }];
  "update:open": [value: boolean];
}>();

const { t } = useI18n();

const force = ref(false);
const replicas = ref(1);

/** Each question starts from the answers it would have if nobody touched it. */
watch(
  () => [props.open, props.kind, props.object] as const,
  () => {
    force.value = false;
    replicas.value = Math.max(0, (props.object ? (numberAt(props.object, "spec", "replicas") ?? 1) : 1) || 0);
  },
  { immediate: true },
);

const kindLabel = computed(() => (props.descriptor ? t(props.descriptor.labelKey) : ""));

const name = computed(() => (props.object ? nameOf(props.object) : ""));

const title = computed(() => {
  switch (props.kind) {
    case "delete":
      return t("k8s.confirm.deleteTitle", { kind: kindLabel.value });
    case "restart":
      return t("k8s.confirm.restartTitle", { kind: kindLabel.value });
    case "scale":
      return t("k8s.confirm.scaleTitle", { kind: kindLabel.value });
    case "cordon":
      return t("k8s.confirm.cordonTitle");
    case "uncordon":
      return t("k8s.confirm.uncordonTitle");
    case "drain":
      return t("k8s.confirm.drainTitle");
    default:
      return "";
  }
});

/** The object, named the way an operator reads it: namespace/name, or just the name at cluster scope. */
const qualified = computed(() => {
  if (props.object === null) {
    return "";
  }
  const namespace = namespaceOf(props.object);
  return namespace === "" ? name.value : `${namespace}/${name.value}`;
});

const message = computed(() => {
  switch (props.kind) {
    case "delete":
      return t("k8s.confirm.deleteMessage", { name: qualified.value });
    case "restart":
      return t("k8s.confirm.restartMessage", { name: qualified.value });
    case "scale":
      return t("k8s.confirm.scaleMessage", { name: qualified.value });
    case "cordon":
      return t("k8s.confirm.cordonMessage", { name: qualified.value });
    case "uncordon":
      return t("k8s.confirm.uncordonMessage", { name: qualified.value });
    case "drain":
      return t("k8s.confirm.drainMessage", { name: qualified.value });
    default:
      return "";
  }
});

const confirmLabel = computed(() => {
  switch (props.kind) {
    case "delete":
      return t("k8s.actions.delete");
    case "restart":
      return t("k8s.actions.restart");
    case "scale":
      return t("k8s.actions.scale");
    case "cordon":
      return t("k8s.actions.cordon");
    case "uncordon":
      return t("k8s.actions.uncordon");
    case "drain":
      return t("k8s.actions.drain");
    default:
      return "";
  }
});

/** Which questions can be taken back: a restart rolls, a scale is a number, the rest do not undo. */
const danger = computed(() => props.kind === "delete" || props.kind === "drain");

const numberValid = computed(() => Number.isInteger(replicas.value) && replicas.value >= 0);

function approve(): void {
  if (props.kind === "scale" && !numberValid.value) {
    return;
  }
  emit("confirm", { force: force.value, replicas: replicas.value });
}
</script>

<template>
  <ConfirmDialog
    :open="props.open"
    :title="title"
    :message="message"
    :confirm-label="confirmLabel"
    :danger="danger"
    :loading="props.busy"
    @update:open="(value: boolean) => emit('update:open', value)"
    @confirm="approve"
  >
    <label v-if="props.kind === 'delete'" class="mt-3 flex items-start gap-2 text-xs">
      <input v-model="force" type="checkbox" class="mt-0.5 size-3 shrink-0 accent-current" />
      <span class="text-muted-foreground">{{ t("k8s.confirm.force") }}</span>
    </label>

    <label v-if="props.kind === 'scale'" class="mt-3 flex items-center gap-2 text-xs">
      <span class="text-muted-foreground">{{ t("k8s.confirm.replicas") }}</span>
      <input
        v-model.number="replicas"
        type="number"
        min="0"
        step="1"
        class="w-20 rounded-md border border-border bg-background px-2 py-1 font-mono text-xs focus:outline-none"
        :aria-label="t('k8s.confirm.replicas')"
      />
    </label>
  </ConfirmDialog>
</template>
