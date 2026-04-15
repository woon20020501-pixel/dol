import Link from "next/link";
import DolHeroImage from "@/components/DolHeroImage";

export const metadata = {
  title: "Not found · Dol",
};

export default function NotFound() {
  return (
    <main className="min-h-screen bg-black text-white flex flex-col items-center justify-center px-6 py-12">
      <DolHeroImage size={200} />

      <h1
        className="mt-12 text-4xl md:text-5xl font-bold text-white text-center leading-[1.05]"
        style={{ letterSpacing: "-0.04em" }}
      >
        Nothing here.
      </h1>

      <p className="mt-4 max-w-md text-center text-[15px] text-white/50">
        This page doesn&apos;t exist. Your Dol is still where you left it.
      </p>

      <Link
        href="/"
        className="mt-10 rounded-full bg-white px-8 py-3 text-[15px] font-semibold text-black hover:bg-white/90 transition-colors"
        style={{
          boxShadow: "0 14px 40px rgba(255,255,255,0.15)",
        }}
      >
        Back to home
      </Link>
    </main>
  );
}
