// Generate a spec-compliant icon.ico that embeds a PNG payload.
// Run with:  node scripts/make-ico.mjs
import { readFileSync, writeFileSync } from "node:fs";
import { resolve, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = dirname(fileURLToPath(import.meta.url));
const root = resolve(__dirname, "..");
const srcPng = resolve(root, "src-tauri/icons/icon.png");
const dstIco = resolve(root, "src-tauri/icons/icon.ico");

const png = readFileSync(srcPng);

// Read PNG dimensions from IHDR (bytes 16..24).
if (png.slice(0, 8).toString("hex") !== "89504e470d0a1a0a") {
  throw new Error("icon.png is not a valid PNG");
}
const width = png.readUInt32BE(16);
const height = png.readUInt32BE(20);
// ICO width/height bytes encode 0 for >= 256.
const w = width >= 256 ? 0 : width;
const h = height >= 256 ? 0 : height;

const ICONDIR_SIZE = 6;
const ICONDIRENTRY_SIZE = 16;
const offset = ICONDIR_SIZE + ICONDIRENTRY_SIZE; // single image

const buf = Buffer.alloc(offset + png.length);

// ICONDIR
buf.writeUInt16LE(0, 0);   // Reserved (must be 0)
buf.writeUInt16LE(1, 2);   // Type 1 = icon
buf.writeUInt16LE(1, 4);   // Image count

// ICONDIRENTRY
buf.writeUInt8(w, 6);      // Width  (0 means 256)
buf.writeUInt8(h, 7);      // Height (0 means 256)
buf.writeUInt8(0, 8);      // ColorCount (0 = no palette)
buf.writeUInt8(0, 9);      // Reserved (MUST be 0)
buf.writeUInt16LE(1, 10);  // Planes
buf.writeUInt16LE(32, 12); // BitCount
buf.writeUInt32LE(png.length, 14); // BytesInRes
buf.writeUInt32LE(offset, 18);     // ImageOffset

png.copy(buf, offset);

writeFileSync(dstIco, buf);
console.log(`wrote ${dstIco} (${buf.length} bytes, payload ${width}x${height} PNG)`);
