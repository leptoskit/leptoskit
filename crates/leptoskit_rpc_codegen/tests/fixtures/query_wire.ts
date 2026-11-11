import { Client } from "./index.ts";
import { presentForAcceptance } from "./live.ts";
const live = presentForAcceptance({note: null, missing: undefined, items: []});
if (JSON.stringify(live) !== '{"note":null,"items":[]}') throw new Error("live presence collapsed");
const requests: {query: string; body: string | null}[] = [];
const client = new Client({fetch: async (url, init) => {
  requests.push({query: new URL(String(url)).search.slice(1), body: init?.body?.toString() ?? null});
  return new Response('"ok"', {headers: {"Content-Type": "application/json"}});
}});
const values = {items: [], labels: {}, note: null, enabled: false, count: 0, text: ""};
// An object whose only fields are undefined is still an empty collection.
await client.probe({...values, labels: {gone: undefined}});
await client.probe({...values, fallback: null});
await client.probe({...values, fallback: ""});
// A hole is null and an undefined field is absent, as in JSON.
await client.probe({...values, items: [, "a +&é", ""], labels: {"key +&é": null, empty: "", gone: undefined}, text: "a +&é"});
console.log(JSON.stringify(requests));
