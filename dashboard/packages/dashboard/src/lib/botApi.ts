/**
 * Typed fetch functions for the Bot Status API (INTERFACES.md section 4).
 * All errors are caught and returned as typed results — never thrown.
 */

import type {
  HealthResponse,
  StatusResponse,
  EventsResponse,
} from "../../../../shared/types/bot-api";

const BOT_API_URL =
  process.env.NEXT_PUBLIC_BOT_API_URL ?? "http://localhost:7777";

async function fetchJson<T>(path: string): Promise<T> {
  const res = await fetch(`${BOT_API_URL}${path}`, {
    cache: "no-store",
  });
  if (!res.ok) {
    throw new Error(`Bot API ${path}: ${res.status} ${res.statusText}`);
  }
  return res.json() as Promise<T>;
}

export async function getHealth(): Promise<HealthResponse> {
  return fetchJson<HealthResponse>("/health");
}

export async function getStatus(): Promise<StatusResponse> {
  return fetchJson<StatusResponse>("/status");
}

export async function getEvents(
  since: number,
  limit: number
): Promise<EventsResponse> {
  return fetchJson<EventsResponse>(
    `/events?since=${since}&limit=${limit}`
  );
}

export async function getMetrics(): Promise<string> {
  const res = await fetch(`${BOT_API_URL}/metrics`, {
    cache: "no-store",
  });
  if (!res.ok) {
    throw new Error(`Bot API /metrics: ${res.status} ${res.statusText}`);
  }
  return res.text();
}
