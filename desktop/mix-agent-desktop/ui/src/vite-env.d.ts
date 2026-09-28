/// <reference types="vite/client" />

declare module "*.css";
declare module "*.svg" {
  const url: string;
  export default url;
}
