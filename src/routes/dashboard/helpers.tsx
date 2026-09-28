import { type ReactNode } from "react";

export function firstText(values: (string | null | undefined)[]): string {
  return values.find((value) => value && value.trim()) ?? "--";
}

export function joinLines(lines: string[]): ReactNode {
  return (
    <div>
      {lines.map((line, index) => (
        <div key={`${line}-${index}`}>{line}</div>
      ))}
    </div>
  );
}
