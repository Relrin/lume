/**
 * The type manifest libghostty-vt publishes about its own ABI.
 *
 * `ghostty_type_json()` describes every public type for the target the module was built
 * for: struct sizes and field offsets, enum values, the bit fields of packed values and
 * the parameters of callbacks. The library states that the manifest, not the C headers,
 * is authoritative for a given build, and that the packed layouts in particular may change
 * between releases. Everything here that touches module memory therefore takes its
 * numbers from the manifest, and a missing descriptor is an error at start-up rather than
 * a wrong read later.
 */

/** A WebAssembly value type. */
export type ValType = "i32" | "i64" | "f32" | "f64";

/** The WebAssembly signature of a function. */
export interface Signature {
  readonly params: readonly ValType[];
  readonly results: readonly ValType[];
}

/** How a C callback type appears to WebAssembly. */
export interface CallbackSignature extends Signature {
  /**
   * True when the C function returns a struct. WebAssembly functions cannot, so the
   * caller passes the address of the result as an extra first parameter and the function
   * returns nothing.
   */
  readonly resultPointer: boolean;
}

/** Where a field sits inside a struct. */
export interface FieldLayout {
  readonly offset: number;
  readonly size: number;
  readonly type: string;
}

/** The size of a struct and the layout of its fields. */
export interface StructLayout {
  readonly size: number;
  readonly align: number;
  readonly fields: Readonly<Record<string, FieldLayout>>;
}

/** Where a field sits inside a packed integer. */
export interface BitField {
  /** Index of the least significant bit. */
  readonly lsb: number;
  readonly width: number;
}

/** The manifest schema this wrapper understands. */
const SCHEMA = 1;

interface RawTypeRef {
  type?: string;
}

interface RawBits {
  lsb?: number;
  width?: number;
  arms?: Record<string, { bits?: Record<string, RawBits> } | null>;
}

interface RawType {
  kind?: string;
  size?: number;
  align?: number;
  type?: string;
  underlying?: string;
  fields?: Record<string, FieldLayout>;
  values?: Record<string, number>;
  bits?: Record<string, RawBits>;
  params?: RawTypeRef[];
  return?: RawTypeRef;
}

interface RawManifest {
  schema?: number;
  abi?: { pointer_size?: number; endian?: string };
  types?: Record<string, RawType>;
}

const PRIMITIVES: Readonly<Record<string, ValType>> = {
  bool: "i32",
  u8: "i32",
  i8: "i32",
  u16: "i32",
  i16: "i32",
  u32: "i32",
  i32: "i32",
  pointer: "i32",
  opaque: "i32",
  u64: "i64",
  i64: "i64",
  f32: "f32",
  f64: "f64",
};

/** A parsed type manifest. */
export class Manifest {
  readonly #types: Record<string, RawType>;

  constructor(json: string) {
    const raw = JSON.parse(json) as RawManifest;
    if (raw.schema !== SCHEMA) {
      throw new Error(
        `libghostty-vt type manifest has schema ${String(raw.schema)}; this wrapper reads schema ${SCHEMA}`,
      );
    }
    // Every offset this wrapper computes and every pointer it stores assumes wasm32.
    if (raw.abi?.pointer_size !== 4 || raw.abi.endian !== "little") {
      throw new Error("libghostty-vt was not built for a 32-bit little-endian target");
    }
    if (!raw.types) throw new Error("libghostty-vt type manifest lists no types");
    this.#types = raw.types;
  }

  /** The layout of a struct. */
  struct(name: string): StructLayout {
    const type = this.#type(name, "struct");
    if (type.size === undefined || type.align === undefined || !type.fields) {
      throw new Error(`libghostty-vt describes ${name} without a layout`);
    }
    return { size: type.size, align: type.align, fields: type.fields };
  }

  /** The offset of one field of a struct. */
  offset(struct: string, field: string): number {
    const found = this.struct(struct).fields[field];
    if (!found) throw new Error(`libghostty-vt struct ${struct} has no field ${field}`);
    return found.offset;
  }

  /** The values of an enum, keyed by name without the C prefix. */
  enumValues(name: string): Readonly<Record<string, number>> {
    const type = this.#type(name, "enum");
    if (!type.values) throw new Error(`libghostty-vt describes ${name} without values`);
    return type.values;
  }

  /** One value of an enum, by its name without the C prefix. */
  enumValue(name: string, value: string): number {
    const found = this.enumValues(name)[value];
    if (found === undefined) throw new Error(`libghostty-vt enum ${name} has no value ${value}`);
    return found;
  }

  /** A bit field of a packed integer. */
  bits(packed: string, field: string): BitField {
    return bitField(this.#type(packed, "packed").bits?.[field], `${packed}.${field}`);
  }

  /**
   * A bit field inside one arm of a tagged field of a packed integer. The position is
   * relative to the tagged field, not to the whole integer.
   */
  armBits(packed: string, field: string, arm: string, name: string): BitField {
    const found = this.#type(packed, "packed").bits?.[field]?.arms?.[arm]?.bits?.[name];
    return bitField(found, `${packed}.${field}.${arm}.${name}`);
  }

  /** The WebAssembly signature of a C callback type. */
  callback(name: string): CallbackSignature {
    const type = this.#type(name, "callback");
    if (!type.params || !type.return) {
      throw new Error(`libghostty-vt describes ${name} without a signature`);
    }
    const params = type.params.map((param, index) => {
      const lowered = this.#scalar(param.type);
      if (!lowered) {
        // The C ABI passes some structs in place and others by address, depending on
        // their fields. No callback this wrapper installs takes one, so the rule is not
        // guessed at here.
        throw new Error(`${name}: parameter ${index} is passed by value as ${String(param.type)}`);
      }
      return lowered;
    });

    if (type.return.type === "void") return { params, results: [], resultPointer: false };
    const result = this.#scalar(type.return.type);
    if (result) return { params, results: [result], resultPointer: false };
    return { params: ["i32", ...params], results: [], resultPointer: true };
  }

  /** The value type a named type lowers to, or `undefined` for a struct or union. */
  #scalar(name: string | undefined): ValType | undefined {
    for (let current = name, depth = 0; current !== undefined && depth < 8; depth++) {
      const primitive = PRIMITIVES[current];
      if (primitive) return primitive;
      const type = this.#types[current];
      if (!type) throw new Error(`libghostty-vt type manifest does not describe ${current}`);
      switch (type.kind) {
        case "opaque":
        case "callback":
          return "i32";
        case "enum":
        case "packed":
          current = type.underlying;
          break;
        case "alias":
          current = type.type;
          break;
        default:
          return undefined;
      }
    }
    throw new Error(`libghostty-vt type ${String(name)} does not resolve to a value type`);
  }

  #type(name: string, kind: string): RawType {
    const type = this.#types[name];
    if (!type) throw new Error(`libghostty-vt type manifest does not describe ${name}`);
    if (type.kind !== kind) {
      throw new Error(`libghostty-vt describes ${name} as ${String(type.kind)}, not ${kind}`);
    }
    return type;
  }
}

function bitField(raw: RawBits | undefined, what: string): BitField {
  if (raw?.lsb === undefined || raw.width === undefined) {
    throw new Error(`libghostty-vt type manifest does not describe the bits of ${what}`);
  }
  return { lsb: raw.lsb, width: raw.width };
}

/**
 * A reader for one bit field of a 64-bit packed value held as two 32-bit halves.
 *
 * Reading the halves and shifting keeps the per-cell path free of `BigInt`, which would
 * allocate for every cell of every row.
 */
export function bitReader(field: BitField): (lo: number, hi: number) => number {
  const { lsb, width } = field;
  if (width < 1 || width > 31 || lsb < 0 || lsb + width > 64) {
    throw new RangeError(`cannot read a bit field at ${lsb} of width ${width}`);
  }
  const mask = 2 ** width - 1;
  if (lsb + width <= 32) return (lo) => (lo >>> lsb) & mask;
  if (lsb >= 32) return (_lo, hi) => (hi >>> (lsb - 32)) & mask;
  // The field straddles the two halves.
  return (lo, hi) => ((lo >>> lsb) | (hi << (32 - lsb))) & mask;
}
