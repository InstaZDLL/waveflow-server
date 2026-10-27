import { useI18n } from "../state/i18n";

/**
 * Text filter shared by the browse pages. It narrows what is already loaded
 * rather than asking the server: `collect` has the whole list in memory, and a
 * per-keystroke round trip would buy nothing.
 */
export function FilterField({
  label,
  value,
  onChange,
}: {
  label: string;
  value: string;
  onChange: (value: string) => void;
}) {
  const { t } = useI18n();
  return (
    <label className="control">
      <span>{label}</span>
      <input
        type="search"
        value={value}
        onChange={(event) => onChange(event.target.value)}
        placeholder={t("browse.filterPlaceholder")}
        aria-label={label}
      />
    </label>
  );
}
