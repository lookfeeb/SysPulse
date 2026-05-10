import i18n from "i18next";
import { initReactI18next } from "react-i18next";
import zhCN from "./zh-CN.json";
import enUS from "./en-US.json";

void i18n.use(initReactI18next).init({
  resources: {
    "zh-CN": { translation: zhCN },
    "en-US": { translation: enUS },
  },
  lng: detectInitialLang(),
  fallbackLng: "zh-CN",
  interpolation: { escapeValue: false },
});

function detectInitialLang(): string {
  const saved = localStorage.getItem("lang");
  if (saved === "zh-CN" || saved === "en-US") return saved;
  if (typeof navigator !== "undefined" && navigator.language?.startsWith("en")) {
    return "en-US";
  }
  return "zh-CN";
}

export default i18n;
