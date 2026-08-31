export function ToggleRow(props: {
  label: string;
  description: string;
  checked: boolean;
  disabled?: boolean;
  onChange: (value: boolean) => void;
}) {
  return (
    <label class={`switch-row ${props.disabled ? "disabled" : ""}`}>
      <span class="switch-copy"><strong>{props.label}</strong><small>{props.description}</small></span>
      <input
        type="checkbox"
        checked={props.checked}
        disabled={props.disabled}
        onChange={(event) => {
          const input = event.currentTarget;
          props.onChange(input.checked);
          input.checked = props.checked;
        }}
      />
      <span class="switch-track"><span /></span>
    </label>
  );
}
