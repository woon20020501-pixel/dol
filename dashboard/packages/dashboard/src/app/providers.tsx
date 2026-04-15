"use client";

import { PrivyProvider } from "@privy-io/react-auth";
import { WagmiProvider } from "@privy-io/wagmi";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { wagmiConfig, baseSepolia } from "@/lib/wagmi";

const queryClient = new QueryClient();

const PRIVY_APP_ID = process.env.NEXT_PUBLIC_PRIVY_APP_ID ?? "";

if (!PRIVY_APP_ID && typeof window !== "undefined") {
  console.warn(
    "[privy] NEXT_PUBLIC_PRIVY_APP_ID is not set. " +
      "Auth will not work. See .env.example."
  );
}

export function Providers({ children }: { children: React.ReactNode }) {
  return (
    <PrivyProvider
      appId={PRIVY_APP_ID}
      config={{
        loginMethods: ["email", "google", "wallet"],
        appearance: {
          theme: "dark",
          accentColor: "#00b4e6",
          logo: undefined,
        },
        // embeddedWallets: create a Privy-managed wallet for users
        // who sign in with email/social and don't already have one.
        // Their key material stays inside Privy's iframe-isolated
        // context; our dashboard JS never sees it directly.
        embeddedWallets: {
          ethereum: {
            createOnLogin: "users-without-wallets",
          },
        },
        // Hard-pin the supported chain set so a compromised build or
        // a swapped RPC can't silently route us to a different chain.
        // wagmi already verifies `chainId` on every writeContract
        // call (Tx confirmation shows the wrong-chain error before
        // any prompt is triggered), but restricting the supported
        // set here is the cheaper upstream guard.
        defaultChain: baseSepolia,
        supportedChains: [baseSepolia],
        // Legal surfaces — Privy renders these links inside its own
        // login modal so a user who enters their credentials on a
        // visually-cloned phishing page is missing our legal links,
        // one small cue that the dialog isn't ours. Also required
        // once we ship real-trade mode for compliance.
        legal: {
          termsAndConditionsUrl: "/legal/terms",
          privacyPolicyUrl: "/legal/privacy",
        },
      }}
    >
      <QueryClientProvider client={queryClient}>
        <WagmiProvider config={wagmiConfig}>
          {children}
        </WagmiProvider>
      </QueryClientProvider>
    </PrivyProvider>
  );
}
