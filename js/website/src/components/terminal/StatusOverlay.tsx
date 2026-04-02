export default function StatusOverlay(props: {
  status: string;
  isError?: boolean;
}) {
  return (
    <div
      class={`absolute inset-0 z-50 flex items-center justify-center bg-[#0a0a0a] font-mono text-sm ${
        props.isError ? "text-red-500" : "text-neutral-400"
      }`}
    >
      {props.status}
    </div>
  );
}
