export interface SwitchProps {
  checked: boolean;
  onChange: (checked: boolean) => void;
  disabled?: boolean;
  "aria-label"?: string;
}

export function Switch({ checked, onChange, disabled, ...aria }: SwitchProps) {
  return (
    <label className="sf-switch">
      <input
        type="checkbox"
        checked={checked}
        disabled={disabled}
        onChange={(e) => onChange(e.target.checked)}
        aria-label={aria["aria-label"]}
      />
      <span className="sf-switch__track">
        <span className="sf-switch__thumb" />
      </span>
    </label>
  );
}
