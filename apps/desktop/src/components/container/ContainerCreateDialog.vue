<script setup lang="ts">
/**
 * A new container, from an image and the few things worth typing.
 *
 * Only the settings that change what the container is are here; the rest are the image's own
 * defaults, and copying them into a form would be a second place for them to be wrong. Ports are
 * two numbers and a protocol because that is what the wire carries — a "8080:80" string would have
 * to be parsed again on the other side, in a place that cannot show which half was mistyped.
 *
 * `--interactive` and `--tty` are offered only where the runtime takes them: the WSL container
 * service does not, and a checkbox that changes nothing is worse than an absent one.
 */
import { Plus, Trash2 } from "@lucide/vue";
import { computed, reactive, ref, watch } from "vue";
import { useI18n } from "vue-i18n";

import Select from "@/components/ui/Select.vue";
import { describeShellError } from "@/i18n/errors";
import { asShellError } from "@/lib/backend/errors";
import { supportsTerminalOptions, useContainerStore } from "@/stores/container";
import type { ContainerPortMapping, ContainerRuntime, ContainerVolume } from "@/types/container";

import ContainerDialog from "./ContainerDialog.vue";

const props = defineProps<{
  open: boolean;
  tabId: string;
  runtime: ContainerRuntime;
  initialImage?: string;
}>();

const emit = defineEmits<{ "update:open": [value: boolean] }>();

const { t } = useI18n();
const containers = useContainerStore();

interface PortRow {
  hostPort: string;
  containerPort: string;
  protocol: "tcp" | "udp";
}

const form = reactive({
  image: "",
  name: "",
  ports: [] as PortRow[],
  volumes: [] as string[],
  env: [] as string[],
  restart: "no",
  command: "",
  interactive: false,
  tty: false,
});

const busy = ref(false);
const failure = ref<string | null>(null);
const imageProblem = ref<string | null>(null);
const portProblem = ref<string | null>(null);
const volumeProblem = ref<string | null>(null);

const terminalOptions = computed(() => supportsTerminalOptions(props.runtime));

/** Docker's own words; the runtime prints these back, so translating them would be misleading. */
const restartOptions = ["no", "always", "unless-stopped", "on-failure"].map((value) => ({
  value,
  label: value,
}));

const protocolOptions = [
  { value: "tcp", label: "tcp" },
  { value: "udp", label: "udp" },
];

function translate(key: string, params?: Record<string, string>): string {
  return params ? t(key, params) : t(key);
}

watch(
  () => props.open,
  (open) => {
    if (!open) {
      return;
    }
    form.image = props.initialImage ?? "";
    form.name = "";
    form.ports = [];
    form.volumes = [];
    form.env = [];
    form.restart = "no";
    form.command = "";
    form.interactive = false;
    form.tty = false;
    busy.value = false;
    failure.value = null;
    imageProblem.value = null;
    portProblem.value = null;
    volumeProblem.value = null;
  },
);

function addPort(): void {
  form.ports.push({ hostPort: "", containerPort: "", protocol: "tcp" });
}

/**
 * Reads a volume the way the Docker CLI writes one.
 *
 * The last colon is the separator: a Windows host path carries one of its own (`C:\data:/data`),
 * and splitting on the first would cut the drive letter off. What is left is two paths, and a
 * mapping with an empty side is refused rather than guessed at.
 */
function parseVolume(text: string): ContainerVolume | null {
  const separator = text.lastIndexOf(":");
  if (separator <= 0 || separator === text.length - 1) {
    return null;
  }
  const source = text.slice(0, separator).trim();
  const destination = text.slice(separator + 1).trim();
  if (source === "" || destination === "") {
    return null;
  }
  return { source, destination };
}

function buildPorts(): ContainerPortMapping[] {
  return form.ports
    .filter((row) => row.hostPort.trim() !== "" || row.containerPort.trim() !== "")
    .map((row) => ({
      hostPort: Number(row.hostPort.trim()),
      containerPort: Number(row.containerPort.trim()),
      protocol: row.protocol,
    }));
}

async function submit(): Promise<void> {
  if (busy.value) {
    return;
  }

  imageProblem.value = null;
  portProblem.value = null;
  volumeProblem.value = null;

  const image = form.image.trim();
  if (image === "") {
    imageProblem.value = t("container.create.imageRequired");
    return;
  }

  const ports = buildPorts();
  for (const row of form.ports) {
    const host = row.hostPort.trim();
    const container = row.containerPort.trim();
    if (host === "" && container === "") {
      continue;
    }
    if (!/^\d+$/.test(host) || !/^\d+$/.test(container)) {
      portProblem.value = t("container.create.portInvalid");
      return;
    }
  }

  const volumes: ContainerVolume[] = [];
  for (const text of form.volumes) {
    if (text.trim() === "") {
      continue;
    }
    const volume = parseVolume(text);
    if (!volume) {
      volumeProblem.value = t("container.create.volumeInvalid");
      return;
    }
    volumes.push(volume);
  }

  busy.value = true;
  try {
    await containers.create(props.tabId, {
      image,
      name: form.name.trim() === "" ? undefined : form.name.trim(),
      ports: ports.length > 0 ? ports : undefined,
      volumes: volumes.length > 0 ? volumes : undefined,
      env: form.env.map((entry) => entry.trim()).filter((entry) => entry !== ""),
      restartPolicy: form.restart,
      command: form.command.trim() === "" ? undefined : form.command.trim(),
      interactive: terminalOptions.value ? form.interactive : false,
      tty: terminalOptions.value ? form.tty : false,
    });
    emit("update:open", false);
  } catch (error) {
    failure.value = describeShellError(translate, asShellError(error, "container_create")).message;
  } finally {
    busy.value = false;
  }
}
</script>

<template>
  <ContainerDialog
    :open="props.open"
    :title="t('container.create.title')"
    :busy="busy"
    @update:open="emit('update:open', $event)"
  >
    <div class="flex flex-col gap-2.5">
      <label class="flex flex-col gap-1">
        <span class="text-[11px] text-muted-foreground">{{ t("container.create.image") }}</span>
        <input
          v-model="form.image"
          type="text"
          spellcheck="false"
          class="rounded-md border border-border bg-background px-2 py-1 font-mono text-xs focus:outline-none"
          :class="imageProblem ? 'border-destructive' : ''"
          :placeholder="t('container.create.imagePlaceholder')"
          @input="imageProblem = null"
        />
        <span v-if="imageProblem" class="text-[10px] text-destructive">{{ imageProblem }}</span>
      </label>

      <label class="flex flex-col gap-1">
        <span class="text-[11px] text-muted-foreground">{{ t("container.create.name") }}</span>
        <input
          v-model="form.name"
          type="text"
          spellcheck="false"
          class="rounded-md border border-border bg-background px-2 py-1 font-mono text-xs focus:outline-none"
        />
      </label>

      <div class="flex flex-col gap-1">
        <span class="text-[11px] text-muted-foreground">{{ t("container.create.ports") }}</span>
        <div v-for="(row, index) in form.ports" :key="index" class="flex items-center gap-1.5">
          <input
            v-model="row.hostPort"
            type="text"
            inputmode="numeric"
            class="w-24 rounded-md border border-border bg-background px-2 py-1 font-mono text-xs focus:outline-none"
            :class="portProblem ? 'border-destructive' : ''"
            :aria-label="t('container.create.portHost')"
            :placeholder="t('container.create.portHost')"
          />
          <span class="text-[11px] text-muted-foreground">:</span>
          <input
            v-model="row.containerPort"
            type="text"
            inputmode="numeric"
            class="w-24 rounded-md border border-border bg-background px-2 py-1 font-mono text-xs focus:outline-none"
            :class="portProblem ? 'border-destructive' : ''"
            :aria-label="t('container.create.portContainer')"
            :placeholder="t('container.create.portContainer')"
          />
          <Select
            :model-value="row.protocol"
            :options="protocolOptions"
            size="compact"
            class="w-16"
            :aria-label="t('container.create.ports')"
            @update:model-value="(value: string) => (row.protocol = value === 'udp' ? 'udp' : 'tcp')"
          />
          <button
            type="button"
            class="grid size-5 place-items-center rounded-sm text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
            :title="t('container.create.removeRow')"
            :aria-label="t('container.create.removeRow')"
            @click="form.ports.splice(index, 1)"
          >
            <Trash2 :size="11" />
          </button>
        </div>
        <button
          type="button"
          class="flex w-fit items-center gap-1 rounded-sm px-1 py-0.5 text-[10px] text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
          @click="addPort()"
        >
          <Plus :size="10" />
          {{ t("container.create.addRow") }}
        </button>
        <span v-if="portProblem" class="text-[10px] text-destructive">{{ portProblem }}</span>
      </div>

      <div class="flex flex-col gap-1">
        <span class="text-[11px] text-muted-foreground">{{ t("container.create.volumes") }}</span>
        <div v-for="(_, index) in form.volumes" :key="index" class="flex items-center gap-1.5">
          <input
            v-model="form.volumes[index]"
            type="text"
            spellcheck="false"
            class="min-w-0 flex-1 rounded-md border border-border bg-background px-2 py-1 font-mono text-xs focus:outline-none"
            :class="volumeProblem ? 'border-destructive' : ''"
            :placeholder="t('container.create.volumePlaceholder')"
            :aria-label="t('container.create.volumes')"
            @input="volumeProblem = null"
          />
          <button
            type="button"
            class="grid size-5 shrink-0 place-items-center rounded-sm text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
            :title="t('container.create.removeRow')"
            :aria-label="t('container.create.removeRow')"
            @click="form.volumes.splice(index, 1)"
          >
            <Trash2 :size="11" />
          </button>
        </div>
        <button
          type="button"
          class="flex w-fit items-center gap-1 rounded-sm px-1 py-0.5 text-[10px] text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
          @click="form.volumes.push('')"
        >
          <Plus :size="10" />
          {{ t("container.create.addRow") }}
        </button>
        <span v-if="volumeProblem" class="text-[10px] text-destructive">{{ volumeProblem }}</span>
      </div>

      <div class="flex flex-col gap-1">
        <span class="text-[11px] text-muted-foreground">{{ t("container.create.env") }}</span>
        <div v-for="(_, index) in form.env" :key="index" class="flex items-center gap-1.5">
          <input
            v-model="form.env[index]"
            type="text"
            spellcheck="false"
            class="min-w-0 flex-1 rounded-md border border-border bg-background px-2 py-1 font-mono text-xs focus:outline-none"
            :placeholder="t('container.create.envPlaceholder')"
            :aria-label="t('container.create.env')"
          />
          <button
            type="button"
            class="grid size-5 shrink-0 place-items-center rounded-sm text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
            :title="t('container.create.removeRow')"
            :aria-label="t('container.create.removeRow')"
            @click="form.env.splice(index, 1)"
          >
            <Trash2 :size="11" />
          </button>
        </div>
        <button
          type="button"
          class="flex w-fit items-center gap-1 rounded-sm px-1 py-0.5 text-[10px] text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
          @click="form.env.push('')"
        >
          <Plus :size="10" />
          {{ t("container.create.addRow") }}
        </button>
      </div>

      <div class="flex flex-col gap-1">
        <span class="text-[11px] text-muted-foreground">{{ t("container.create.restart") }}</span>
        <Select
          v-model="form.restart"
          :options="restartOptions"
          class="w-44"
          :aria-label="t('container.create.restart')"
        />
      </div>

      <label class="flex flex-col gap-1">
        <span class="text-[11px] text-muted-foreground">{{ t("container.create.command") }}</span>
        <input
          v-model="form.command"
          type="text"
          spellcheck="false"
          class="rounded-md border border-border bg-background px-2 py-1 font-mono text-xs focus:outline-none"
        />
      </label>

      <div v-if="terminalOptions" class="flex flex-col gap-1">
        <span class="text-[11px] text-muted-foreground">{{ t("container.create.terminal") }}</span>
        <label class="flex items-center gap-1.5 text-[11px]">
          <input v-model="form.interactive" type="checkbox" class="size-3 accent-[var(--primary)]" />
          {{ t("container.create.interactive") }}
        </label>
        <label class="flex items-center gap-1.5 text-[11px]">
          <input v-model="form.tty" type="checkbox" class="size-3 accent-[var(--primary)]" />
          {{ t("container.create.tty") }}
        </label>
        <p class="text-[10px] leading-relaxed text-muted-foreground">{{ t("container.create.terminalHint") }}</p>
      </div>

      <p v-if="failure" class="text-[11px] text-destructive select-text">{{ failure }}</p>
    </div>

    <template #footer>
      <button
        type="button"
        class="rounded-md border border-border px-3 py-1.5 text-xs transition-colors hover:bg-accent disabled:cursor-not-allowed disabled:opacity-40"
        :disabled="busy"
        @click="emit('update:open', false)"
      >
        {{ t("common.action.cancel") }}
      </button>
      <button
        type="button"
        class="rounded-md bg-primary px-3 py-1.5 text-xs text-primary-foreground transition-colors hover:bg-primary/90 disabled:cursor-not-allowed disabled:opacity-60"
        :disabled="busy"
        @click="submit()"
      >
        {{ t("container.create.submit") }}
      </button>
    </template>
  </ContainerDialog>
</template>
