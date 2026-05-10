import { Alert } from "antd";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { isAdmin } from "@/ipc";

export default function AdminBanner() {
  const { t } = useTranslation();
  const [elevated, setElevated] = useState<boolean | null>(null);

  useEffect(() => {
    isAdmin().then(setElevated).catch(() => setElevated(true));
  }, []);

  if (elevated !== false) return null;

  return (
    <Alert
      type="error"
      banner
      showIcon
      message={t("admin.notElevatedTitle")}
      description={t("admin.notElevatedDesc")}
      style={{ marginBottom: 12 }}
    />
  );
}
