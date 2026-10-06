import { useSyncExternalStore } from "react";
import { getLocale, subscribeLanguage } from "./i18n.mjs";

export const useLanguage = () =>
  useSyncExternalStore(subscribeLanguage, getLocale, getLocale);
