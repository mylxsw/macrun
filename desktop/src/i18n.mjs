import english from "./locales/en.json" with { type: "json" };

/** @typedef {"system" | "zh-CN" | "en"} Language */
/** @typedef {"zh-CN" | "en"} Locale */
const listeners = new Set();
/** @type {Language} */
let language = "system";
/** @type {Locale | undefined} */
let nativeLocale;

/** @param {string | undefined} value @returns {Locale} */
export function resolveLocale(value) {
  return /^zh(?:[-_]|$)/i.test(value || "") ? "zh-CN" : "en";
}
/** @returns {Locale} */
export function getLocale() {
  return language === "system"
    ? nativeLocale || resolveLocale(globalThis.navigator?.language)
    : language;
}
export const getLanguage = () => language;

/** Apply only confirmed preferences; persistence is owned by the native app.
 * @param {Language} next
 * @param {string | undefined} effectiveLocale
 */
export function setLanguage(next, effectiveLocale) {
  language = ["system", "zh-CN", "en"].includes(next) ? next : "system";
  nativeLocale = effectiveLocale ? resolveLocale(effectiveLocale) : undefined;
  if (globalThis.document) document.documentElement.lang = getLocale();
  for (const listener of listeners) listener();
}
/** @param {() => void} listener */
export function subscribeLanguage(listener) {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

/** Chinese source text is the catalog key and the Chinese fallback.
 * An optional "context::" prefix distinguishes words with different meanings.
 * Placeholders are substituted once, so user content is never translated
 * or interpreted as another placeholder.
 * @param {string} source
 * @param {...unknown} values
 */
export function tr(source, ...values) {
  const fallback = source.includes("::")
    ? source.split("::").slice(1).join("::")
    : source;
  const text =
    getLocale() === "en" && Object.hasOwn(english, source)
      ? english[source]
      : fallback;
  return text.replace(/\{(\d+)\}/g, (placeholder, index) =>
    Number(index) < values.length
      ? String(values[Number(index)] ?? "")
      : placeholder,
  );
}

export function refreshSystemLanguage() {
  if (language === "system") setLanguage("system");
}
