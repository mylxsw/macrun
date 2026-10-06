import { tr } from "./i18n.mjs";
import React, { useId, useLayoutEffect, useRef, useState } from "react";
import { ChevronDown, ChevronUp } from "lucide-react";

/** Key by task ID so selecting another task starts with a compact preview. */
export function CommandPreview({ command }: { command: string }) {
  const id = useId();
  const text = useRef<HTMLPreElement>(null);
  const [expanded, setExpanded] = useState(false);
  const [overflows, setOverflows] = useState(false);

  useLayoutEffect(() => {
    const element = text.current;
    if (!element) return;
    const measure = () => {
      const lineHeight = parseFloat(getComputedStyle(element).lineHeight);
      setOverflows(element.scrollHeight > lineHeight * 3 + 1);
    };
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(element);
    return () => observer.disconnect();
  }, [command]);

  return (
    <div className="v4-command-preview">
      <pre
        ref={text}
        id={id}
        className={`mono v4-command-text${expanded ? " expanded" : ""}`}
        aria-label={tr("完整命令")}
      >
        {command}
      </pre>
      {overflows && (
        <button
          className="link v4-command-toggle"
          aria-expanded={expanded}
          aria-controls={id}
          onClick={() => setExpanded((value) => !value)}
        >
          {expanded ? <ChevronUp size={13} /> : <ChevronDown size={13} />}
          {expanded ? tr("收起命令") : tr("展开命令")}
        </button>
      )}
    </div>
  );
}
