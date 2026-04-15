/**
 * Dashboard layout — Apple Pro dark theme for the operator dashboard.
 * Uses #0a0a0b almost-black (not pure black) for a premium feel.
 */
export default function DashboardLayout({
  children,
}: {
  children: React.ReactNode;
}) {
  return (
    <div className="dark min-h-screen bg-[#0a0a0b] text-[#f5f5f7]">
      {children}
    </div>
  );
}
