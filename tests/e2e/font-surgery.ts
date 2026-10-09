/**
 * In-memory sfnt surgery that derives TEST FACES of a font family from the
 * committed OFL fixture (#830). It is the TypeScript mirror of the Rust
 * `state/stream_fonts/test_fonts.rs` helpers (`with_names`, `with_os2_italic`).
 * A derived face only ever lives in memory; nothing here is written to disk or
 * committed (a modified OFL font must not carry the original name, and the
 * real mislabelled families on SNV/PP are commercial fonts).
 *
 * sfnt layout: a 12-byte header (`numTables` at byte 4), then one 16-byte
 * record per table: `tag[4] checksum[4] offset[4] length[4]`, big-endian.
 * Browsers' OpenType sanitiser ignores checksums, so nothing is re-summed.
 */

const SFNT_HEADER_LEN = 12;
const TABLE_RECORD_LEN = 16;

function recordPos(font: Buffer, tag: string): number {
  const numTables = font.readUInt16BE(4);
  for (let i = 0; i < numTables; i++) {
    const pos = SFNT_HEADER_LEN + i * TABLE_RECORD_LEN;
    if (font.toString("latin1", pos, pos + 4) === tag) return pos;
  }
  throw new Error(`font has no ${tag} table`);
}

function tableOffset(font: Buffer, tag: string): number {
  return font.readUInt32BE(recordPos(font, tag) + 8);
}

function decodeUtf16be(bytes: Buffer): string {
  return Buffer.from(bytes).swap16().toString("utf16le");
}

function encodeUtf16be(text: string): Buffer {
  return Buffer.from(text, "utf16le").swap16();
}

/** The font's Unicode (0) / Windows (3) names by id, first record per id. */
function existingNames(font: Buffer): Map<number, string> {
  const table = tableOffset(font, "name");
  const count = font.readUInt16BE(table + 2);
  const storage = table + font.readUInt16BE(table + 4);
  const names = new Map<number, string>();
  for (let i = 0; i < count; i++) {
    const record = table + 6 + i * 12;
    const platform = font.readUInt16BE(record);
    if (platform !== 0 && platform !== 3) continue;
    const id = font.readUInt16BE(record + 6);
    const length = font.readUInt16BE(record + 8);
    const start = storage + font.readUInt16BE(record + 10);
    if (!names.has(id)) {
      names.set(id, decodeUtf16be(font.subarray(start, start + length)));
    }
  }
  return names;
}

/** A format-0 `name` table: Windows / Unicode BMP / en-US records by id. */
function buildNameTable(names: Map<number, string>): Buffer {
  const ids = [...names.keys()].sort((a, b) => a - b);
  const strings = ids.map((id) => encodeUtf16be(names.get(id) ?? ""));
  const header = Buffer.alloc(6 + 12 * ids.length);
  header.writeUInt16BE(0, 0);
  header.writeUInt16BE(ids.length, 2);
  header.writeUInt16BE(6 + 12 * ids.length, 4);
  let offset = 0;
  ids.forEach((id, i) => {
    const record = 6 + i * 12;
    header.writeUInt16BE(3, record);
    header.writeUInt16BE(1, record + 2);
    header.writeUInt16BE(0x0409, record + 4);
    header.writeUInt16BE(id, record + 6);
    header.writeUInt16BE(strings[i].length, record + 8);
    header.writeUInt16BE(offset, record + 10);
    offset += strings[i].length;
  });
  return Buffer.concat([header, ...strings]);
}

/**
 * Copy of `font` whose `name` table carries `names` (name id → text). Each entry
 * replaces that id's string; every other Unicode/Windows name is kept. The new
 * table is appended at a 4-byte aligned offset and the `name` record repointed
 * at it; the old table stays as an unused gap, so no other table moves.
 */
export function withNames(font: Buffer, names: Record<number, string>): Buffer {
  const merged = existingNames(font);
  for (const [id, text] of Object.entries(names)) merged.set(Number(id), text);
  const table = buildNameTable(merged);
  const pad = (4 - (font.length % 4)) % 4;
  const out = Buffer.concat([font, Buffer.alloc(pad), table]);
  const record = recordPos(out, "name");
  out.writeUInt32BE(font.length + pad, record + 8);
  out.writeUInt32BE(table.length, record + 12);
  return out;
}

/** Copy of `font` flagged italic in OS/2 `fsSelection` (ITALIC set, REGULAR cleared). */
export function withItalicFlag(font: Buffer): Buffer {
  const out = Buffer.from(font);
  const at = tableOffset(out, "OS/2") + 62;
  const flags = (out.readUInt16BE(at) | 0x0001) & ~0x0040;
  out.writeUInt16BE(flags & 0xffff, at);
  return out;
}
