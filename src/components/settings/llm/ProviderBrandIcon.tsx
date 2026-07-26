import type { FC, SVGProps } from "react";
/* Color when available; Mono-only brands have no Color variant in @lobehub/icons. */
import Ai21 from "@lobehub/icons/es/Ai21/components/Mono";
import Ai360 from "@lobehub/icons/es/Ai360/components/Color";
import Alibaba from "@lobehub/icons/es/Alibaba/components/Color";
import Anthropic from "@lobehub/icons/es/Anthropic/components/Mono";
import Aws from "@lobehub/icons/es/Aws/components/Color";
import Azure from "@lobehub/icons/es/Azure/components/Color";
import AzureAI from "@lobehub/icons/es/AzureAI/components/Color";
import Baichuan from "@lobehub/icons/es/Baichuan/components/Color";
import Baidu from "@lobehub/icons/es/Baidu/components/Color";
import Bedrock from "@lobehub/icons/es/Bedrock/components/Color";
import ByteDance from "@lobehub/icons/es/ByteDance/components/Color";
import Cerebras from "@lobehub/icons/es/Cerebras/components/Color";
import ChatGLM from "@lobehub/icons/es/ChatGLM/components/Color";
import Claude from "@lobehub/icons/es/Claude/components/Color";
import Cloudflare from "@lobehub/icons/es/Cloudflare/components/Color";
import Cohere from "@lobehub/icons/es/Cohere/components/Color";
import DeepInfra from "@lobehub/icons/es/DeepInfra/components/Color";
import DeepSeek from "@lobehub/icons/es/DeepSeek/components/Color";
import Doubao from "@lobehub/icons/es/Doubao/components/Color";
import Fal from "@lobehub/icons/es/Fal/components/Mono";
import Fireworks from "@lobehub/icons/es/Fireworks/components/Color";
import Flux from "@lobehub/icons/es/Flux/components/Mono";
import Gemini from "@lobehub/icons/es/Gemini/components/Color";
import Gemma from "@lobehub/icons/es/Gemma/components/Color";
import Google from "@lobehub/icons/es/Google/components/Color";
import Grok from "@lobehub/icons/es/Grok/components/Mono";
import Groq from "@lobehub/icons/es/Groq/components/Mono";
import HuggingFace from "@lobehub/icons/es/HuggingFace/components/Color";
import Hunyuan from "@lobehub/icons/es/Hunyuan/components/Color";
import Hyperbolic from "@lobehub/icons/es/Hyperbolic/components/Color";
import Inflection from "@lobehub/icons/es/Inflection/components/Mono";
import InternLM from "@lobehub/icons/es/InternLM/components/Color";
import Kimi from "@lobehub/icons/es/Kimi/components/Color";
import Kling from "@lobehub/icons/es/Kling/components/Color";
import Liquid from "@lobehub/icons/es/Liquid/components/Mono";
import Luma from "@lobehub/icons/es/Luma/components/Color";
import Meta from "@lobehub/icons/es/Meta/components/Color";
import Microsoft from "@lobehub/icons/es/Microsoft/components/Color";
import Midjourney from "@lobehub/icons/es/Midjourney/components/Mono";
import Minimax from "@lobehub/icons/es/Minimax/components/Color";
import Mistral from "@lobehub/icons/es/Mistral/components/Color";
import Moonshot from "@lobehub/icons/es/Moonshot/components/Mono";
import Nebius from "@lobehub/icons/es/Nebius/components/Mono";
import Novita from "@lobehub/icons/es/Novita/components/Color";
import Nvidia from "@lobehub/icons/es/Nvidia/components/Color";
import Ollama from "@lobehub/icons/es/Ollama/components/Mono";
import OpenAI from "@lobehub/icons/es/OpenAI/components/Mono";
import OpenRouter from "@lobehub/icons/es/OpenRouter/components/Mono";
import PaLM from "@lobehub/icons/es/PaLM/components/Color";
import Perplexity from "@lobehub/icons/es/Perplexity/components/Color";
import Qwen from "@lobehub/icons/es/Qwen/components/Color";
import Replicate from "@lobehub/icons/es/Replicate/components/Mono";
import Runway from "@lobehub/icons/es/Runway/components/Mono";
import SambaNova from "@lobehub/icons/es/SambaNova/components/Color";
import SiliconCloud from "@lobehub/icons/es/SiliconCloud/components/Color";
import Snowflake from "@lobehub/icons/es/Snowflake/components/Color";
import Spark from "@lobehub/icons/es/Spark/components/Color";
import Stability from "@lobehub/icons/es/Stability/components/Color";
import Stepfun from "@lobehub/icons/es/Stepfun/components/Color";
import Tencent from "@lobehub/icons/es/Tencent/components/Color";
import TencentCloud from "@lobehub/icons/es/TencentCloud/components/Color";
import Together from "@lobehub/icons/es/Together/components/Color";
import Upstage from "@lobehub/icons/es/Upstage/components/Color";
import VertexAI from "@lobehub/icons/es/VertexAI/components/Color";
import Vidu from "@lobehub/icons/es/Vidu/components/Color";
import Volcengine from "@lobehub/icons/es/Volcengine/components/Color";
import Wenxin from "@lobehub/icons/es/Wenxin/components/Color";
import WorkersAI from "@lobehub/icons/es/WorkersAI/components/Color";
import XAI from "@lobehub/icons/es/XAI/components/Mono";
import Yi from "@lobehub/icons/es/Yi/components/Color";
import ZeroOne from "@lobehub/icons/es/ZeroOne/components/Color";
import Zhipu from "@lobehub/icons/es/Zhipu/components/Color";
import {
  isCustomAvatarImage,
  resolveBrandIconId,
  resolveManageGroupIconId,
  resolveModelBrandIconId,
  shortProviderMark,
} from "./modelServices";

type BrandIconComponent = FC<
  SVGProps<SVGSVGElement> & { size?: number | string }
>;

/**
 * Prefer Color SVG; fall back to Mono when the brand has no Color variant.
 * Deep-import components only — icon index Avatar/Combine pull antd.
 */
const BRAND_ICONS: Record<string, BrandIconComponent> = {
  ai21: Ai21,
  ai360: Ai360,
  alibaba: Alibaba,
  amazon: Aws,
  anthropic: Anthropic,
  aws: Aws,
  azure: Azure,
  azureai: AzureAI,
  baichuan: Baichuan,
  baidu: Baidu,
  bedrock: Bedrock,
  bytedance: ByteDance,
  cerebras: Cerebras,
  chatglm: ChatGLM,
  claude: Claude,
  cloudflare: Cloudflare,
  cohere: Cohere,
  deepinfra: DeepInfra,
  deepseek: DeepSeek,
  doubao: Doubao,
  fal: Fal,
  fireworksai: Fireworks,
  flux: Flux,
  gemini: Gemini,
  gemma: Gemma,
  google: Google,
  grok: Grok,
  groq: Groq,
  huggingface: HuggingFace,
  hunyuan: Hunyuan,
  hyperbolic: Hyperbolic,
  inflection: Inflection,
  internlm: InternLM,
  kimi: Kimi,
  kling: Kling,
  liquid: Liquid,
  luma: Luma,
  meta: Meta,
  microsoft: Microsoft,
  midjourney: Midjourney,
  minimax: Minimax,
  mistral: Mistral,
  moonshot: Moonshot,
  nebius: Nebius,
  novita: Novita,
  nvidia: Nvidia,
  ollama: Ollama,
  openai: OpenAI,
  openrouter: OpenRouter,
  palm: PaLM,
  perplexity: Perplexity,
  qwen: Qwen,
  replicate: Replicate,
  runway: Runway,
  sambanova: SambaNova,
  siliconcloud: SiliconCloud,
  snowflake: Snowflake,
  spark: Spark,
  stability: Stability,
  stepfun: Stepfun,
  tencent: Tencent,
  tencentcloud: TencentCloud,
  togetherai: Together,
  upstage: Upstage,
  vertexai: VertexAI,
  vidu: Vidu,
  volcengine: Volcengine,
  wenxin: Wenxin,
  workersai: WorkersAI,
  xai: XAI,
  yi: Yi,
  zeroone: ZeroOne,
  zhipu: Zhipu,
};

export function ProviderBrandIcon({
  provider,
  sdk,
  model,
  group,
  src,
  fallback,
  size = 18,
  className,
}: {
  /** Provider / brand name. */
  provider?: string | null;
  /** SDK id fallback for brand resolution. */
  sdk?: string | null;
  /** Full model id (may include `vendor/name`). */
  model?: string | null;
  /** Manage-models group id. */
  group?: string | null;
  /** Custom avatar image URL / data URL. */
  src?: string | null;
  fallback?: string;
  size?: number;
  className?: string;
}) {
  if (isCustomAvatarImage(src)) {
    return (
      <span className={`${className ?? ""} has-image`.trim()}>
        <img src={src!.trim()} alt="" />
      </span>
    );
  }

  const brandId =
    (group ? resolveManageGroupIconId(group) : "") ||
    (model ? resolveModelBrandIconId(model) : "") ||
    resolveBrandIconId(provider) ||
    resolveBrandIconId(sdk);

  const Icon = brandId ? BRAND_ICONS[brandId] : undefined;
  if (Icon) {
    return (
      <span className={`${className ?? ""} has-image`.trim()}>
        <Icon size={size} />
      </span>
    );
  }

  return <span className={className}>{fallback || "·"}</span>;
}

export function ProviderAvatarDisplay({
  name,
  avatar,
  sdk,
  className = "model-provider-avatar",
  size = 22,
}: {
  name: string;
  avatar?: string;
  sdk?: string;
  className?: string;
  size?: number;
}) {
  return (
    <ProviderBrandIcon
      className={className}
      src={avatar}
      provider={name}
      sdk={sdk}
      size={size}
      fallback={shortProviderMark(name)}
    />
  );
}
