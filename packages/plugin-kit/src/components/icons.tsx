/** The few marks the kit's own controls draw: lines in the colour of the text around them. */

function Mark({ size = 12, children }: { size?: number | undefined; children: React.ReactNode }) {
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 16 16"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
    >
      {children}
    </svg>
  );
}

export function CheckMark({ size }: { size?: number | undefined }) {
  return (
    <Mark size={size}>
      <path d="M3.5 8.5l3 3 6-7" />
    </Mark>
  );
}

export function DashMark({ size }: { size?: number | undefined }) {
  return (
    <Mark size={size}>
      <path d="M4 8h8" />
    </Mark>
  );
}

export function ChevronRight({ size }: { size?: number | undefined }) {
  return (
    <Mark size={size}>
      <path d="M6 3.5l4.5 4.5L6 12.5" />
    </Mark>
  );
}

export function ChevronDown({ size }: { size?: number | undefined }) {
  return (
    <Mark size={size}>
      <path d="M3.5 6l4.5 4.5L12.5 6" />
    </Mark>
  );
}

/** Joins the class names that are there. */
export function cx(...names: (string | false | null | undefined)[]): string {
  return names.filter(Boolean).join(" ");
}
