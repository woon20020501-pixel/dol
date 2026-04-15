import type { Config } from "tailwindcss";

const config: Config = {
  darkMode: "class",
  content: [
    "./src/pages/**/*.{js,ts,jsx,tsx,mdx}",
    "./src/components/**/*.{js,ts,jsx,tsx,mdx}",
    "./src/app/**/*.{js,ts,jsx,tsx,mdx}",
  ],
  theme: {
    extend: {
      colors: {
        border: "hsl(var(--border))",
        input: "hsl(var(--input))",
        ring: "hsl(var(--ring))",
        background: "hsl(var(--background))",
        foreground: "hsl(var(--foreground))",
        primary: {
          DEFAULT: "hsl(var(--primary))",
          foreground: "hsl(var(--primary-foreground))",
        },
        secondary: {
          DEFAULT: "hsl(var(--secondary))",
          foreground: "hsl(var(--secondary-foreground))",
        },
        destructive: {
          DEFAULT: "hsl(var(--destructive))",
          foreground: "hsl(var(--destructive-foreground))",
        },
        muted: {
          DEFAULT: "hsl(var(--muted))",
          foreground: "hsl(var(--muted-foreground))",
        },
        accent: {
          DEFAULT: "hsl(var(--accent))",
          foreground: "hsl(var(--accent-foreground))",
        },
        popover: {
          DEFAULT: "hsl(var(--popover))",
          foreground: "hsl(var(--popover-foreground))",
        },
        card: {
          DEFAULT: "hsl(var(--card))",
          foreground: "hsl(var(--card-foreground))",
        },
        // Pacifica brand colors
        pacific: {
          50: "#e6f9ff",
          100: "#b3efff",
          200: "#80e4ff",
          300: "#4dd9ff",
          400: "#1aceff",
          500: "#00b4e6",
          600: "#008db4",
          700: "#006682",
          800: "#004050",
          900: "#001a1f",
        },
        carry: {
          green: "#30d158",
          red: "#ff453a",
          amber: "#ff9f0a",
        },
        // Apple Pro dark surfaces
        dark: {
          bg: "#0a0a0b",
          surface: "#141416",
          "surface-2": "#1c1c1f",
          border: "#2a2a2d",
          "border-strong": "#3a3a3e",
          primary: "#f5f5f7",
          secondary: "#86868b",
          tertiary: "#5a5a5f",
        },
        // Landing page accent colors
        senior: {
          DEFAULT: "#2dd4bf",
          dark: "#14b8a6",
        },
        junior: {
          DEFAULT: "#f59e0b",
          dark: "#d97706",
        },
        surface: {
          DEFAULT: "#f5f5f7",
          2: "#fafafa",
        },
        apple: {
          primary: "#1d1d1f",
          secondary: "#86868b",
          border: "#d2d2d7",
        },
      },
      fontFamily: {
        sans: ["var(--font-inter)", "system-ui", "sans-serif"],
        mono: ["var(--font-jetbrains)", "Consolas", "monospace"],
        heading: ["var(--font-inter)", "system-ui", "sans-serif"],
      },
      borderRadius: {
        lg: "var(--radius)",
        md: "calc(var(--radius) - 2px)",
        sm: "calc(var(--radius) - 4px)",
      },
      keyframes: {
        "accordion-down": {
          from: { height: "0" },
          to: { height: "var(--radix-accordion-content-height)" },
        },
        "accordion-up": {
          from: { height: "var(--radix-accordion-content-height)" },
          to: { height: "0" },
        },
      },
      animation: {
        "accordion-down": "accordion-down 0.2s ease-out",
        "accordion-up": "accordion-up 0.2s ease-out",
      },
    },
  },
  plugins: [require("tailwindcss-animate")],
};
export default config;
