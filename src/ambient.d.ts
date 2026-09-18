/// <reference types="svelte" />
/// <reference types="vite/client" />

declare module "*.svelte" {
  import type { ComponentType } from "svelte";
  const component: ComponentType;
  export default component;
}

declare const __BUILD_TIMESTAMP__: string;
declare const __BUILD_NUMBER__: string;
