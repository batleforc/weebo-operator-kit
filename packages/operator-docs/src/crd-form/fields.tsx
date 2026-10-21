"use client";

import { useId, useState, type ReactNode } from "react";
import { Callout } from "fumadocs-ui/components/callout";
import {
  asArray,
  asRecord,
  defaultForSchema,
  discriminatedVariants,
  isDiscriminatedUnion,
  variantSchema,
} from "./schema-utils";
import type { FieldSchema } from "./types";

const inputClass =
  "w-full border border-(--rule-strong) bg-(--ground-sunk) px-3 py-2 text-[15px] text-fd-foreground placeholder:text-fd-muted-foreground focus:border-(--accent)";

const labelClass = "text-[12px] font-semibold uppercase tracking-[0.14em] text-fd-muted-foreground";

const ghostButtonClass = "ctl";

function splitDescription(description?: string): [string, string] {
  if (!description) return ["", ""];
  const [first = "", ...rest] = description.split(/\n\s*\n/);
  return [first.replace(/\s*\n\s*/g, " ").trim(), rest.join("\n\n").trim()];
}

// The id of the field's description, for the input's `aria-describedby`;
// undefined when there is none to point at.
function descriptionId(id: string, description?: string): string | undefined {
  return splitDescription(description)[0] ? `${id}-desc` : undefined;
}

// Joins `aria-describedby` ids, undefined when none is left.
function describedBy(...ids: (string | undefined)[]): string | undefined {
  return ids.filter(Boolean).join(" ") || undefined;
}

// The first paragraph, clamped to two lines; a longer one or the following
// paragraphs unfold under it (a <details>, keyboard reachable). The id goes
// on the first paragraph, which is what `aria-describedby` reads.
function FieldDescription({ id, description }: { id: string; description?: string | undefined }) {
  const [summary, rest] = splitDescription(description);
  if (!summary) return null;
  const textClass = "text-[12px] text-fd-muted-foreground";
  if (!rest && summary.length <= 100) {
    return (
      <p id={id} className={`line-clamp-2 ${textClass}`}>
        {summary}
      </p>
    );
  }
  return (
    <details className={`group ${textClass}`}>
      <summary id={id} className="line-clamp-2 cursor-pointer list-none group-open:line-clamp-none">
        {summary}
      </summary>
      {rest ? <p className="mt-1 whitespace-pre-line">{rest}</p> : null}
    </details>
  );
}

function Caption({ name, required }: { name: string; required?: boolean | undefined }) {
  return (
    <>
      {name}
      {required ? <span className="text-fd-primary"> *</span> : null}
    </>
  );
}

// The name is the input's <label>, its description under it.
function FieldHeader({
  name,
  required,
  description,
  htmlFor,
}: {
  name: string;
  required?: boolean | undefined;
  description?: string | undefined;
  htmlFor: string;
}) {
  return (
    <div className="flex flex-col gap-0.5">
      <label htmlFor={htmlFor} className={labelClass}>
        <Caption name={name} required={required} />
      </label>
      <FieldDescription id={`${htmlFor}-desc`} description={description} />
    </div>
  );
}

function FieldGroup({
  name,
  required,
  description,
  children,
}: {
  name: string;
  required?: boolean | undefined;
  description?: string | undefined;
  children: ReactNode;
}) {
  const id = useId();
  return (
    <fieldset
      aria-describedby={descriptionId(id, description)}
      className="flex flex-col gap-3 border border-(--rule-soft) p-3"
    >
      <legend className={`px-1 ${labelClass}`}>
        <Caption name={name} required={required} />
      </legend>
      <FieldDescription id={`${id}-desc`} description={description} />
      {children}
    </fieldset>
  );
}

export interface FieldProps {
  name: string;
  schema: FieldSchema;
  required?: boolean | undefined;
  value: unknown;
  onChange: (value: unknown) => void;
}

// What each input below receives: the value already read as its type, the
// field's `onChange` passed as is (it takes any value).
interface InputProps<T> {
  name: string;
  schema: FieldSchema;
  required?: boolean | undefined;
  value: T;
  onChange: (v: T) => void;
}

export function Field({ name, schema, required, value, onChange }: FieldProps) {
  if (isDiscriminatedUnion(schema)) {
    return (
      <DiscriminatedUnionField
        name={name}
        schema={schema}
        required={required}
        value={asRecord(value)}
        onChange={onChange}
      />
    );
  }
  if (schema.type === "array") {
    return (
      <ArrayField
        name={name}
        schema={schema}
        required={required}
        value={asArray(value)}
        onChange={onChange}
      />
    );
  }
  if (schema.type === "object" && schema.additionalProperties) {
    return (
      <MapField
        name={name}
        schema={schema}
        required={required}
        value={asRecord(value)}
        onChange={onChange}
      />
    );
  }
  if (schema.type === "object" && schema.properties) {
    return (
      <FieldGroup name={name} required={required} description={schema.description}>
        <ObjectFields
          schema={schema}
          value={value}
          onChange={onChange}
        />
      </FieldGroup>
    );
  }
  if (schema.type === "object") {
    return <RawJsonField name={name} schema={schema} required={required} value={value} onChange={onChange} />;
  }
  if (schema.enum) {
    return (
      <SelectField
        name={name}
        schema={schema}
        required={required}
        value={typeof value === "string" ? value : ""}
        onChange={onChange}
        options={schema.enum}
      />
    );
  }
  if (schema.type === "boolean") {
    return (
      <BooleanField
        name={name}
        schema={schema}
        value={Boolean(value)}
        onChange={onChange}
      />
    );
  }
  if (schema.type === "integer" || schema.type === "number") {
    return (
      <NumberField
        name={name}
        schema={schema}
        required={required}
        value={Number(value ?? 0)}
        onChange={onChange}
      />
    );
  }
  return (
    <StringField
      name={name}
      schema={schema}
      required={required}
      value={typeof value === "string" ? value : ""}
      onChange={onChange}
    />
  );
}

export function ObjectFields({
  schema,
  value,
  onChange,
  omitKeys,
  collapseOptional,
  priorityKeys,
}: {
  schema: FieldSchema;
  value: unknown;
  onChange: (value: Record<string, unknown>) => void;
  // Lets the caller take a property out of the generic recursion to render
  // it itself (e.g. CrdForm merging `spec.name` into the metadata.name
  // input) — only meant for the root call, nested ObjectFields calls never
  // pass this.
  omitKeys?: string[] | undefined;
  // Root call only: optional fields fold under "More settings", so what must
  // be filled in is what the reader sees first.
  collapseOptional?: boolean | undefined;
  // Optional fields to keep upfront anyway (the ones a guided choice is about).
  priorityKeys?: string[] | undefined;
}) {
  const properties = schema.properties ?? {};
  const required = new Set(schema.required ?? []);
  const obj = asRecord(value);
  const omit = new Set(omitKeys ?? []);
  const entries = Object.entries(properties).filter(([key]) => !omit.has(key));
  // Required fields, then priority ones, then the rest; each keeps schema order.
  const priority = new Set(priorityKeys ?? []);
  const mandatory = [
    ...entries.filter(([key]) => required.has(key)),
    ...entries.filter(([key]) => !required.has(key) && priority.has(key)),
  ];
  const optional = entries.filter(([key]) => !required.has(key) && !priority.has(key));
  const render = ([key, childSchema]: [string, FieldSchema]) => (
    <Field
      key={key}
      name={key}
      schema={childSchema}
      required={required.has(key)}
      value={obj[key]}
      onChange={(v) => onChange({ ...obj, [key]: v })}
    />
  );
  return (
    <div className="flex flex-col gap-4">
      {mandatory.map(render)}
      {collapseOptional && optional.length > 0 ? (
        <details className="group border border-(--rule-soft)">
          <summary className="cursor-pointer list-none px-3 py-2 text-[12px] font-semibold uppercase tracking-[0.1em] text-fd-muted-foreground hover:text-fd-foreground">
            <span className="group-open:hidden">+ </span>
            <span className="hidden group-open:inline">− </span>
            More settings ({optional.length})
          </summary>
          <div className="flex flex-col gap-4 border-t border-(--rule-soft) p-3">{optional.map(render)}</div>
        </details>
      ) : (
        optional.map(render)
      )}
    </div>
  );
}

function StringField({ name, schema, required, value, onChange }: InputProps<string>) {
  const id = useId();
  return (
    <div className="flex flex-col gap-1">
      <FieldHeader name={name} required={required} description={schema.description} htmlFor={id} />
      <input
        id={id}
        aria-required={required}
        aria-describedby={descriptionId(id, schema.description)}
        type="text"
        className={inputClass}
        value={value}
        placeholder={schema.nullable ? "(optional)" : ""}
        onChange={(e) => onChange(e.target.value)}
      />
    </div>
  );
}

function NumberField({ name, schema, required, value, onChange }: InputProps<number>) {
  const id = useId();
  return (
    <div className="flex flex-col gap-1">
      <FieldHeader name={name} required={required} description={schema.description} htmlFor={id} />
      <input
        id={id}
        aria-required={required}
        aria-describedby={descriptionId(id, schema.description)}
        type="number"
        className={inputClass}
        value={Number.isFinite(value) ? value : 0}
        onChange={(e) => onChange(e.target.valueAsNumber || 0)}
      />
    </div>
  );
}

function BooleanField({ name, schema, value, onChange }: Omit<InputProps<boolean>, "required">) {
  const id = useId();
  return (
    <div className="flex items-start gap-2">
      <input
        id={id}
        type="checkbox"
        aria-describedby={descriptionId(id, schema.description)}
        checked={value}
        onChange={(e) => onChange(e.target.checked)}
        className="mt-0.5 size-4 border border-(--rule-strong) bg-(--ground-sunk) accent-(--accent)"
      />
      <FieldHeader name={name} description={schema.description} htmlFor={id} />
    </div>
  );
}

function SelectField({
  name,
  schema,
  required,
  value,
  onChange,
  options,
}: InputProps<string> & { options: string[] }) {
  const id = useId();
  return (
    <div className="flex flex-col gap-1">
      <FieldHeader name={name} required={required} description={schema.description} htmlFor={id} />
      <select
        id={id}
        aria-required={required}
        aria-describedby={descriptionId(id, schema.description)}
        className={inputClass}
        value={value}
        onChange={(e) => onChange(e.target.value)}
      >
        {!value ? <option value="">-- select --</option> : null}
        {options.map((opt) => (
          <option key={opt} value={opt}>
            {opt}
          </option>
        ))}
      </select>
    </div>
  );
}

function RawJsonField({ name, schema, required, value, onChange }: InputProps<unknown>) {
  const id = useId();
  const [parseError, setParseError] = useState<string | null>(null);
  const text = typeof value === "string" ? value : JSON.stringify(value ?? {}, null, 2);
  const description = schema.description ?? "Free-form JSON, passed through to the remote object as-is.";
  return (
    <div className="flex flex-col gap-1">
      <FieldHeader
        name={name}
        required={required}
        description={description}
        htmlFor={id}
      />
      <textarea
        id={id}
        aria-required={required}
        aria-invalid={parseError !== null}
        aria-describedby={describedBy(descriptionId(id, description), parseError === null ? undefined : `${id}-error`)}
        className={`${inputClass} min-h-24 font-mono ${parseError === null ? "" : "border-(--accent)"}`}
        defaultValue={text}
        onBlur={(e) => {
          // Invalid JSON keeps the last valid value, and says so.
          try {
            const parsed: unknown = JSON.parse(e.target.value);
            onChange(parsed);
            setParseError(null);
          } catch (err: unknown) {
            setParseError(err instanceof Error ? err.message : String(err));
          }
        }}
      />
      {parseError === null ? null : (
        <p id={`${id}-error`} role="alert" className="text-[12px] text-fd-primary">
          Not valid JSON, the manifest keeps the last valid value: {parseError}
        </p>
      )}
    </div>
  );
}

function ArrayField({ name, schema, required, value, onChange }: InputProps<unknown[]>) {
  const itemSchema = schema.items ?? {};
  const singular = name.endsWith("s") ? name.slice(0, -1) : name;
  return (
    <FieldGroup name={name} required={required} description={schema.description}>
      <div className="flex flex-col gap-3">
        {value.map((item, index) => (
          <div key={index} className="flex items-start gap-2 border border-dashed border-(--rule-soft) p-2">
            <div className="flex-1">
              <Field
                name={`${name}[${index}]`}
                schema={itemSchema}
                value={item}
                onChange={(v) => onChange(value.map((it, i) => (i === index ? v : it)))}
              />
            </div>
            <button
              type="button"
              onClick={() => onChange(value.filter((_, i) => i !== index))}
              className={ghostButtonClass}
              aria-label={`remove ${name}[${index}]`}
            >
              remove
            </button>
          </div>
        ))}
        <button
          type="button"
          onClick={() => onChange([...value, defaultForSchema(itemSchema)])}
          className={`${ghostButtonClass} self-start`}
        >
          + add {singular}
        </button>
      </div>
    </FieldGroup>
  );
}

function MapField({ name, schema, required, value, onChange }: InputProps<Record<string, unknown>>) {
  const entries = Object.entries(value).map(([key, val]): [string, string] => [key, typeof val === "string" ? val : ""]);

  const updateEntry = (index: number, key: string, val: string) => {
    const next = [...entries];
    next[index] = [key, val];
    onChange(Object.fromEntries(next.filter(([k]) => k !== "")));
  };

  return (
    <FieldGroup name={name} required={required} description={schema.description}>
      <div className="flex flex-col gap-2">
        {entries.map(([key, val], index) => (
          <div key={index} className="flex items-center gap-2">
            <input
              className={inputClass}
              placeholder="key"
              aria-label={`${name}: key of entry ${index + 1}`}
              defaultValue={key}
              onBlur={(e) => updateEntry(index, e.target.value, val)}
            />
            <input
              className={inputClass}
              placeholder="value"
              aria-label={`${name}: value of entry ${index + 1}`}
              defaultValue={val}
              onBlur={(e) => updateEntry(index, key, e.target.value)}
            />
            <button
              type="button"
              onClick={() => onChange(Object.fromEntries(entries.filter((_, i) => i !== index)))}
              className={ghostButtonClass}
              aria-label={`remove ${name} entry ${index + 1}`}
            >
              remove
            </button>
          </div>
        ))}
        <button
          type="button"
          onClick={() => onChange({ ...value, [`key${entries.length}`]: "" })}
          className={`${ghostButtonClass} self-start`}
        >
          + add entry
        </button>
      </div>
    </FieldGroup>
  );
}

function DiscriminatedUnionField({ name, schema, required, value, onChange }: InputProps<Record<string, unknown>>) {
  const variants = discriminatedVariants(schema);
  const kind = typeof value.kind === "string" ? value.kind : variants[0];
  const vSchema = kind === undefined ? undefined : variantSchema(schema, kind);

  return (
    <FieldGroup name={name} required={required} description={schema.description}>
      <div className="flex flex-col gap-4">
        <SelectField
          name="kind"
          schema={schema.properties?.kind ?? {}}
          required
          value={kind ?? ""}
          onChange={(newKind) => {
            const newVariantSchema = variantSchema(schema, newKind);
            onChange({
              kind: newKind,
              ...(newVariantSchema ? { [newKind]: defaultForSchema(newVariantSchema) } : {}),
            });
          }}
          options={variants}
        />
        {vSchema && kind !== undefined ? (
          <ObjectFields
            schema={vSchema}
            value={value[kind]}
            onChange={(v) => onChange({ kind, [kind]: v })}
          />
        ) : kind ? (
          <Callout type="warn" title={`"${kind}" is a schema-only stub in v1`}>
            Accepted here for forward compatibility, but the reconciler rejects it with an explicit
            error rather than applying it silently.
          </Callout>
        ) : null}
      </div>
    </FieldGroup>
  );
}
