import { useState } from "react";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Power, RotateCw } from "lucide-react";
import { api } from "@/lib/api";
import { errorMessage } from "@/lib/errors";
import { t } from "@/lib/i18n";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { LOCAL_RUNTIMES_QUERY_KEY, useLocalRuntimes } from "@/lib/queries";
import { localRuntimeForModel } from "@/lib/localRuntime";
import type { ModelView, ProviderView } from "@/lib/types";

export { localRuntimeForModel };

export function LocalRuntimeStatusChip({
  modelId,
  models,
  providers,
}: {
  modelId: string;
  models: ModelView[];
  providers: ProviderView[];
}) {
  const slug = localRuntimeForModel(modelId, models, providers);
  // Keep the query hooks out of the render path when the selection is not
  // local: Composer is rendered by tests and previews without a
  // QueryClientProvider, and there is nothing for the chip to show anyway.
  if (!slug) return null;
  return <LocalRuntimeChipBody slug={slug} />;
}

function LocalRuntimeChipBody({ slug }: { slug: string }) {
  const queryClient = useQueryClient();
  const runtimes = useLocalRuntimes();
  const runtime = runtimes.data?.find((entry) => entry.slug === slug);
  const [error, setError] = useState<string | null>(null);
  const start = useMutation({
    mutationFn: (value: string) => api.localRuntimeStart(value),
    onMutate: () => setError(null),
    onSuccess: () =>
      queryClient.invalidateQueries({ queryKey: LOCAL_RUNTIMES_QUERY_KEY }),
    onError: (err) => setError(errorMessage(err)),
  });

  if (!runtime) return null;
  // Once the runtime is up (by this button or another path) the error is
  // history and must not hide the online state.
  if (runtime.online) {
    return (
      <Badge
        tone={runtime.state === "degraded" ? "warning" : "success"}
        size="sm"
        title={runtime.endpoint}
        className="max-w-40 truncate"
      >
        {runtime.label} · {t("オンライン")}
      </Badge>
    );
  }
  if (error) {
    return (
      <Button
        size="sm"
        variant="ghost-danger"
        className="max-w-48"
        title={error}
        disabled={start.isPending}
        onClick={() => start.mutate(runtime.slug)}
      >
        {start.isPending ? <RotateCw size={12} /> : <Power size={12} />}
        <span className="truncate">
          {runtime.label} · {t("起動失敗・再試行")}
        </span>
      </Button>
    );
  }
  if (!runtime.can_start) {
    return (
      <Badge tone="warning" size="sm" title={runtime.error ?? runtime.endpoint}>
        {runtime.label} · {t("オフライン")}
      </Badge>
    );
  }
  return (
    <Button
      size="sm"
      variant="ghost"
      disabled={start.isPending}
      title={runtime.error ?? t("ローカルランタイムを起動")}
      onClick={() => start.mutate(runtime.slug)}
    >
      {start.isPending ? <RotateCw size={12} /> : <Power size={12} />}
      {t("起動")}
    </Button>
  );
}
