import { redirect } from "@sveltejs/kit";

export function load(): never {
  // SvelteKit redirects are thrown as framework control flow.
  // eslint-disable-next-line @typescript-eslint/only-throw-error
  throw redirect(307, "/transactions");
}
