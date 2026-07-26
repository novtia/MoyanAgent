import {
  startTransition,
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { useTranslation } from "react-i18next";
import { api } from "../../api/tauri";
import { useSettings } from "../../store/settings";
import {
  catalogPricesFromProviders,
  estimateCost,
  useUsagePricing,
} from "../../store/usagePricing";
import type {
  DailyUsageRow,
  TokenUsageEventRow,
  TokenUsageSummary,
  ToolUsageRow,
} from "../../types";
import { EventsTable, PAGE_SIZE } from "./EventsTable";
import { ModelDist } from "./ModelDist";
import { PricingCard } from "./PricingCard";
import { ScopeControl } from "./ScopeControl";
import { StatStrip } from "./StatStrip";
import { ToolDist } from "./ToolDist";
import { TrendChart } from "./TrendChart";
import {
  fillDailyGaps,
  formatMdShort,
  localDateStr,
  previousRange,
  rangeForScope,
  type UsageScope,
} from "./format";

interface OverviewData {
  summary: TokenUsageSummary;
  prevSummary: TokenUsageSummary | null;
  daily: DailyUsageRow[];
  tools: ToolUsageRow[];
}

export function UsageView() {
  const { t } = useTranslation();
  const [scope, setScope] = useState<UsageScope>("7d");
  const [page, setPage] = useState(0);
  const [eventKind, setEventKind] = useState<string | null>(null);
  const [modelFilter, setModelFilter] = useState<string | null>(null);

  const [overview, setOverview] = useState<OverviewData | null>(null);
  const [events, setEvents] = useState<TokenUsageEventRow[]>([]);
  const [hasMore, setHasMore] = useState(false);
  const [refreshing, setRefreshing] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const prices = useUsagePricing((s) => s.prices);
  const costEnabled = useUsagePricing((s) => s.enabled);
  const currency = useUsagePricing((s) => s.currency);
  const ensureModels = useUsagePricing((s) => s.ensureModels);
  const modelServices = useSettings((s) => s.settings?.model_services);
  const catalogPrices = useMemo(
    () => catalogPricesFromProviders(modelServices),
    [modelServices],
  );

  const range = useMemo(() => rangeForScope(scope), [scope]);
  const prev = useMemo(() => previousRange(scope), [scope]);

  const overviewSeq = useRef(0);
  const eventsSeq = useRef(0);

  const onScopeChange = useCallback((next: UsageScope) => {
    startTransition(() => {
      setScope(next);
      setPage(0);
    });
  }, []);

  const onEventKindChange = useCallback((kind: string | null) => {
    setEventKind(kind);
    setPage(0);
  }, []);

  const onModelChange = useCallback((model: string | null) => {
    setModelFilter(model);
    setPage(0);
  }, []);

  useEffect(() => {
    const seq = ++overviewSeq.current;
    setRefreshing(true);
    setError(null);

    void (async () => {
      try {
        const [sum, dailyRows, toolRows, prevSum] = await Promise.all([
          api.getTokenUsageSummary({
            fromMs: range.fromMs,
            toMs: range.toMs,
          }),
          api.getTokenUsageDaily({
            fromMs: range.fromMs,
            toMs: range.toMs,
          }),
          api.getTokenUsageByTool({
            fromMs: range.fromMs,
            toMs: range.toMs,
          }),
          prev
            ? api.getTokenUsageSummary({
                fromMs: prev.fromMs,
                toMs: prev.toMs,
              })
            : Promise.resolve(null),
        ]);
        if (seq !== overviewSeq.current) return;

        const daily = fillDailyGaps(dailyRows, range.fromMs, range.toMs);
        setOverview({
          summary: sum,
          prevSummary: prevSum,
          daily,
          tools: toolRows,
        });
        ensureModels(sum.by_model.map((m) => m.model).filter(Boolean));
      } catch (e) {
        if (seq !== overviewSeq.current) return;
        console.error(e);
        setError(t("usage.loadError"));
      } finally {
        if (seq === overviewSeq.current) setRefreshing(false);
      }
    })();
  }, [range, prev, ensureModels, t]);

  useEffect(() => {
    const seq = ++eventsSeq.current;
    void (async () => {
      try {
        const rows = await api.listTokenUsageEvents({
          fromMs: range.fromMs,
          toMs: range.toMs,
          eventKind,
          model: modelFilter,
          limit: PAGE_SIZE,
          offset: page * PAGE_SIZE,
        });
        if (seq !== eventsSeq.current) return;
        setEvents(rows);
        setHasMore(rows.length >= PAGE_SIZE);
      } catch (e) {
        if (seq !== eventsSeq.current) return;
        console.error(e);
      }
    })();
  }, [range, eventKind, modelFilter, page]);

  const summary = overview?.summary ?? null;
  const prevSummary = overview?.prevSummary ?? null;
  const daily = overview?.daily ?? [];
  const tools = overview?.tools ?? [];

  const cost = useMemo(() => {
    if (!summary || !costEnabled) return null;
    return estimateCost(summary.by_model, prices, catalogPrices);
  }, [summary, costEnabled, prices, catalogPrices]);

  const toolErrorCount = useMemo(
    () => tools.reduce((s, r) => s + r.error_count, 0),
    [tools],
  );

  const modelNames = useMemo(
    () => (summary?.by_model ?? []).map((m) => m.model).filter(Boolean),
    [summary],
  );

  const fromLabel =
    daily.length > 0
      ? formatMdShort(daily[0].date)
      : range.fromMs
        ? formatMdShort(localDateStr(range.fromMs))
        : "—";
  const toLabel =
    daily.length > 0
      ? formatMdShort(daily[daily.length - 1].date)
      : formatMdShort(localDateStr(Date.now()));

  return (
    <main className={`usage${refreshing ? " is-refreshing" : ""}`}>
      <div className="usage-panel">
        <header className="usage-page-head">
          <div className="usage-page-head-main">
            <h1 className="usage-page-title">{t("usage.title")}</h1>
            <p className="usage-page-subtitle">{t("usage.subtitle")}</p>
          </div>
          <ScopeControl value={scope} onChange={onScopeChange} />
        </header>

        {error ? <div className="usage-error">{error}</div> : null}

        <StatStrip
          summary={summary}
          prevSummary={prevSummary}
          toolErrorCount={toolErrorCount}
          cost={cost}
          costEnabled={costEnabled}
          currency={currency}
        />

        <TrendChart rows={daily} fromLabel={fromLabel} toLabel={toLabel} />
        <ModelDist rows={summary?.by_model ?? []} />
        <ToolDist rows={tools} />
        <EventsTable
          events={events}
          page={page}
          hasMore={hasMore}
          models={modelNames}
          eventKind={eventKind}
          model={modelFilter}
          onEventKindChange={onEventKindChange}
          onModelChange={onModelChange}
          onPageChange={setPage}
          loading={refreshing && !overview}
        />
        <PricingCard models={modelNames} catalogPrices={catalogPrices} />

        <div className="usage-privacy">{t("usage.privacy")}</div>
      </div>
    </main>
  );
}
