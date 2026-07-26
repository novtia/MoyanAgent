import type { FC, SVGProps } from "react";
import Bing from "@lobehub/icons/es/Bing/components/Color";
import Google from "@lobehub/icons/es/Google/components/Color";

type IconComp = FC<SVGProps<SVGSVGElement> & { size?: number | string }>;

/** DuckDuckGo-inspired mark for local scraping. */
const DuckDuckGoIcon: IconComp = ({ size = "1em", style, ...rest }) => (
  <svg
    width={size}
    height={size}
    viewBox="0 0 24 24"
    style={{ flex: "none", lineHeight: 1, ...style }}
    {...rest}
  >
    <circle cx="12" cy="12" r="12" fill="#DE5833" />
    <circle cx="12" cy="10.2" r="5.2" fill="#FFF" />
    <ellipse cx="10.2" cy="9.6" rx="1.15" ry="1.35" fill="#222" />
    <ellipse cx="13.8" cy="9.6" rx="1.15" ry="1.35" fill="#222" />
    <path
      fill="#65BC46"
      d="M7.2 14.8c1.4 2.4 3.1 3.6 4.8 3.6s3.4-1.2 4.8-3.6c-1.3.9-3 .9-4.8.9s-3.5 0-4.8-.9z"
    />
  </svg>
);

/** Tavily brand mark (approx.). */
const TavilyIcon: IconComp = ({ size = "1em", style, ...rest }) => (
  <svg
    width={size}
    height={size}
    viewBox="0 0 24 24"
    style={{ flex: "none", lineHeight: 1, ...style }}
    {...rest}
  >
    <rect width="24" height="24" rx="6" fill="#4636E3" />
    <path
      fill="#fff"
      d="M6.5 7.2h11c.4 0 .7.3.7.7v1.1c0 .4-.3.7-.7.7H13.4v7.4c0 .4-.3.7-.7.7h-1.4c-.4 0-.7-.3-.7-.7V9.7H6.5c-.4 0-.7-.3-.7-.7V7.9c0-.4.3-.7.7-.7z"
    />
  </svg>
);

const KIND_ICONS: Record<string, IconComp> = {
  local: DuckDuckGoIcon,
  duckduckgo: DuckDuckGoIcon,
  tavily: TavilyIcon,
  serper: Google,
  bing: Bing,
};

export function SearchBrandIcon({
  kind,
  size = 18,
  className,
  fallback,
}: {
  kind?: string | null;
  size?: number;
  className?: string;
  fallback?: string;
}) {
  const key = (kind ?? "").trim().toLowerCase();
  const Icon = key ? KIND_ICONS[key] : undefined;
  if (Icon) {
    return (
      <span className={`${className ?? ""} has-image`.trim()} aria-hidden>
        <Icon size={size} />
      </span>
    );
  }
  return (
    <span className={className} aria-hidden>
      {fallback || "·"}
    </span>
  );
}
