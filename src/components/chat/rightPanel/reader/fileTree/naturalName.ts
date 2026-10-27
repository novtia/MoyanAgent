/** Natural filename order. Mirrors `natural_name.rs` for the export dialog's default name. */

const DIGIT: Record<string, number> = {
  零: 0,
  〇: 0,
  一: 1,
  壹: 1,
  二: 2,
  贰: 2,
  貳: 2,
  两: 2,
  兩: 2,
  三: 3,
  叁: 3,
  參: 3,
  四: 4,
  肆: 4,
  五: 5,
  伍: 5,
  六: 6,
  陆: 6,
  陸: 6,
  七: 7,
  柒: 7,
  八: 8,
  捌: 8,
  九: 9,
  玖: 9,
};

const UNIT: Record<string, number> = {
  十: 10,
  拾: 10,
  百: 100,
  佰: 100,
  千: 1000,
  仟: 1000,
  万: 10000,
  萬: 10000,
};

type Token = { kind: "text"; text: string } | { kind: "num"; value: number };

function isCjkNumeral(ch: string): boolean {
  return Object.prototype.hasOwnProperty.call(DIGIT, ch) || Object.prototype.hasOwnProperty.call(UNIT, ch);
}

function parseChinese(raw: string): number | null {
  const chars = Array.from(raw);
  if (chars.length === 0) return null;
  const hasUnit = chars.some((ch) => UNIT[ch] != null);
  if (!hasUnit) {
    let n = 0;
    for (const ch of chars) {
      const d = DIGIT[ch];
      if (d == null) return null;
      n = n * 10 + d;
    }
    return n;
  }
  let total = 0;
  let section = 0;
  let number = 0;
  for (const ch of chars) {
    const digit = DIGIT[ch];
    const unit = UNIT[ch];
    if (digit != null) {
      number = number * 10 + digit;
    } else if (unit != null) {
      if (unit === 10000) {
        if (number === 0 && section === 0) number = 1;
        section += number;
        total += section * 10000;
        section = 0;
        number = 0;
      } else {
        if (number === 0) number = 1;
        section += number * unit;
        number = 0;
      }
    } else {
      return null;
    }
  }
  return total + section + number;
}

function tokenize(value: string): Token[] {
  const chars = Array.from(value);
  const out: Token[] = [];
  let i = 0;
  while (i < chars.length) {
    const ch = chars[i];
    if (ch >= "0" && ch <= "9") {
      let j = i + 1;
      while (j < chars.length && chars[j] >= "0" && chars[j] <= "9") j += 1;
      out.push({ kind: "num", value: Number(chars.slice(i, j).join("")) });
      i = j;
    } else if (isCjkNumeral(ch)) {
      let j = i + 1;
      while (j < chars.length && isCjkNumeral(chars[j])) j += 1;
      const raw = chars.slice(i, j).join("");
      const parsed = parseChinese(raw);
      out.push(parsed == null ? { kind: "text", text: raw } : { kind: "num", value: parsed });
      i = j;
    } else {
      let j = i + 1;
      while (j < chars.length && !(chars[j] >= "0" && chars[j] <= "9") && !isCjkNumeral(chars[j])) {
        j += 1;
      }
      out.push({ kind: "text", text: chars.slice(i, j).join("") });
      i = j;
    }
  }
  return out;
}

function cmpText(a: string, b: string): number {
  const left = Array.from(a);
  const right = Array.from(b);
  const n = Math.min(left.length, right.length);
  for (let i = 0; i < n; i += 1) {
    const x = left[i] >= "A" && left[i] <= "Z" ? left[i].toLowerCase() : left[i];
    const y = right[i] >= "A" && right[i] <= "Z" ? right[i].toLowerCase() : right[i];
    if (x < y) return -1;
    if (x > y) return 1;
  }
  return left.length - right.length;
}

export function compareNaturalName(a: string, b: string): number {
  const ta = tokenize(a);
  const tb = tokenize(b);
  const n = Math.min(ta.length, tb.length);
  for (let i = 0; i < n; i += 1) {
    const x = ta[i];
    const y = tb[i];
    if (x.kind === "num" && y.kind === "num") {
      if (x.value !== y.value) return x.value < y.value ? -1 : 1;
    } else if (x.kind === "text" && y.kind === "text") {
      const ord = cmpText(x.text, y.text);
      if (ord !== 0) return ord;
    } else {
      return x.kind === "num" ? -1 : 1;
    }
  }
  if (ta.length !== tb.length) return ta.length - tb.length;
  return cmpText(a, b);
}

export function txtStem(name: string): string {
  return name.replace(/\.txt$/i, "");
}
