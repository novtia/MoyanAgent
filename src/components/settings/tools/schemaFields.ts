export interface ToolSchemaField {
  name: string;
  type: string;
  description: string;
  required: boolean;
}

function fieldType(value: unknown): string {
  if (!value || typeof value !== "object") return "any";
  const node = value as { type?: unknown; enum?: unknown[] };
  if (Array.isArray(node.enum) && node.enum.length > 0) {
    return node.enum.map(String).join(" | ");
  }
  if (typeof node.type === "string" && node.type.trim()) return node.type;
  if (Array.isArray(node.type) && node.type.length > 0) {
    return node.type.map(String).join(" | ");
  }
  return "any";
}

function fieldDescription(value: unknown): string {
  if (!value || typeof value !== "object") return "";
  const node = value as { description?: unknown };
  return typeof node.description === "string" ? node.description : "";
}

export function schemaFields(schema: unknown): ToolSchemaField[] {
  if (!schema || typeof schema !== "object") return [];
  const root = schema as {
    properties?: Record<string, unknown>;
    required?: unknown;
  };
  const properties = root.properties;
  if (!properties || typeof properties !== "object") return [];
  const required = new Set(
    Array.isArray(root.required)
      ? root.required.filter((name): name is string => typeof name === "string")
      : [],
  );
  return Object.entries(properties).map(([name, value]) => ({
    name,
    type: fieldType(value),
    description: fieldDescription(value),
    required: required.has(name),
  }));
}
