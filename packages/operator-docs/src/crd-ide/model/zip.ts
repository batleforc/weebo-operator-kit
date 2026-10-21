// A stored (uncompressed) zip of a few small text files: the manifests are
// kilobytes, a deflate library would weigh more than it saves.

const CRC_TABLE = Array.from({ length: 256 }, (_, n) => {
  let c = n;
  for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
  return c >>> 0;
});

export function crc32(bytes: Uint8Array): number {
  let crc = 0xffffffff;
  for (const byte of bytes) crc = (CRC_TABLE[(crc ^ byte) & 0xff] ?? 0) ^ (crc >>> 8);
  return (crc ^ 0xffffffff) >>> 0;
}

class Writer {
  private chunks: Uint8Array[] = [];
  length = 0;
  bytes(b: Uint8Array) {
    this.chunks.push(b);
    this.length += b.length;
  }
  u16(n: number) {
    this.bytes(new Uint8Array([n & 0xff, (n >>> 8) & 0xff]));
  }
  u32(n: number) {
    this.bytes(new Uint8Array([n & 0xff, (n >>> 8) & 0xff, (n >>> 16) & 0xff, (n >>> 24) & 0xff]));
  }
  concat(): Uint8Array {
    const out = new Uint8Array(this.length);
    let at = 0;
    for (const c of this.chunks) {
      out.set(c, at);
      at += c.length;
    }
    return out;
  }
}

/** The zip of `files` (UTF-8 names and contents, no date). */
export function zip(files: { name: string; content: string }[]): Uint8Array {
  const utf8 = new TextEncoder();
  const out = new Writer();
  const central = new Writer();
  for (const file of files) {
    const name = utf8.encode(file.name);
    const data = utf8.encode(file.content);
    const crc = crc32(data);
    const offset = out.length;
    // Local header: version 2.0, UTF-8 names (bit 11), stored, no date.
    out.u32(0x04034b50);
    out.u16(20);
    out.u16(0x0800);
    out.u16(0);
    out.u32(0);
    out.u32(crc);
    out.u32(data.length);
    out.u32(data.length);
    out.u16(name.length);
    out.u16(0);
    out.bytes(name);
    out.bytes(data);
    central.u32(0x02014b50);
    central.u16(20);
    central.u16(20);
    central.u16(0x0800);
    central.u16(0);
    central.u32(0);
    central.u32(crc);
    central.u32(data.length);
    central.u32(data.length);
    central.u16(name.length);
    central.u16(0);
    central.u16(0);
    central.u16(0);
    central.u16(0);
    central.u32(0);
    central.u32(offset);
    central.bytes(name);
  }
  const centralOffset = out.length;
  const centralBytes = central.concat();
  out.bytes(centralBytes);
  out.u32(0x06054b50);
  out.u16(0);
  out.u16(0);
  out.u16(files.length);
  out.u16(files.length);
  out.u32(centralBytes.length);
  out.u32(centralOffset);
  out.u16(0);
  return out.concat();
}
