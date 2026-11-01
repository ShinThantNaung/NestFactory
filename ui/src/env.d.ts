declare module '*?worker' {
  const WorkerFactory: { new (): Worker };
  export default WorkerFactory;
}

declare module 'monaco-editor/languages/definitions/typescript/typescript' {
  import type { languages } from 'monaco-editor';
  export const language: languages.IMonarchLanguage;
  export const conf: languages.LanguageConfiguration;
}

declare module '*.css';
declare module '@fontsource-variable/geist';
declare module '@fontsource-variable/geist-mono';
