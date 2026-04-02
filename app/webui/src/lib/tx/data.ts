import { tr } from "@repo/hyphen-i18n";

import type { ApiTxDetail } from "$lib";

import type { TransactionRenderer } from "./types";
import type { TransactionPayloadField } from "./types";

function readByte(script: Uint8Array, index: number): number | null {
  return index >= 0 && index < script.length ? (script[index] ?? null) : null;
}

function readPushDataLength(script: Uint8Array, index: number): { nextIndex: number; length: number } | null {
  const opcode = readByte(script, index);

  if (opcode == null) {
    return null;
  }

  if (opcode <= 0x4b) {
    return {
      nextIndex: index + 1,
      length: opcode,
    };
  }

  if (opcode === 0x4c) {
    const length = readByte(script, index + 1);

    if (length == null) {
      return null;
    }

    return {
      nextIndex: index + 2,
      length,
    };
  }

  if (opcode === 0x4d) {
    const lengthLow = readByte(script, index + 1);
    const lengthHigh = readByte(script, index + 2);

    if (lengthLow == null || lengthHigh == null) {
      return null;
    }

    return {
      nextIndex: index + 3,
      length: lengthLow | (lengthHigh << 8),
    };
  }

  if (opcode === 0x4e) {
    const length0 = readByte(script, index + 1);
    const length1 = readByte(script, index + 2);
    const length2 = readByte(script, index + 3);
    const length3 = readByte(script, index + 4);

    if (length0 == null || length1 == null || length2 == null || length3 == null) {
      return null;
    }

    return {
      nextIndex: index + 5,
      length: length0 | (length1 << 8) | (length2 << 16) | (length3 << 24),
    };
  }

  return null;
}

function decodeHexBytes(hex: string): Uint8Array | null {
  if (hex.length === 0 || hex.length % 2 !== 0 || /[^0-9a-f]/iu.test(hex)) {
    return null;
  }

  return Uint8Array.from(
    Array.from({ length: hex.length / 2 }, (_, index): number =>
      Number.parseInt(hex.slice(index * 2, index * 2 + 2), 16)
    )
  );
}

function encodeHexBytes(bytes: Uint8Array): string {
  return Array.from(bytes, (byte): string => byte.toString(16).padStart(2, "0")).join("");
}

function extractOpReturnHex(scriptHex: string): string | null {
  const script = decodeHexBytes(scriptHex);

  if (script == null || script.length === 0 || script[0] !== 0x6a) {
    return null;
  }

  const chunks: number[] = [];
  let index = 1;

  while (index < script.length) {
    const pushData = readPushDataLength(script, index);

    if (pushData == null) {
      return null;
    }

    const start = pushData.nextIndex;
    const end = start + pushData.length;

    if (end > script.length) {
      return null;
    }

    chunks.push(...script.slice(start, end));
    index = end;
  }

  if (chunks.length === 0) {
    return null;
  }

  return encodeHexBytes(Uint8Array.from(chunks));
}

function renderDataPayload(tx: ApiTxDetail): readonly TransactionPayloadField[] {
  const fields = tx.vout.flatMap((output): readonly TransactionPayloadField[] => {
    const opReturnHex = extractOpReturnHex(output.scriptPubKey.hex);

    if (opReturnHex == null) {
      return [];
    }

    return [
      {
        label: "Hex",
        value: opReturnHex,
        mono: true,
      },
    ];
  });

  if (fields.length > 0) {
    return fields;
  }

  return [
    {
      label: "Hex",
      value: tr("Empty"),
      muted: true,
    },
  ];
}

export const dataRenderer: TransactionRenderer = {
  type: "data",
  label: "Data",
  tone: "data",
  renderMode: "payload",
  matches: (tx): boolean => tx.txType === "data",
  parsePayload: renderDataPayload,
};
