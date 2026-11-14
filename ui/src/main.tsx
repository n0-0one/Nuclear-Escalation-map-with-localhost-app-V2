import { render } from "preact";
import "@fontsource/barlow-semi-condensed/latin-500.css";
import "@fontsource/barlow-semi-condensed/latin-600.css";
import "@fontsource/jetbrains-mono/latin-400.css";
import "@fontsource/jetbrains-mono/latin-600.css";
import "./style.css";
import { App } from "./App";
import { Overlay } from "./Overlay";
import { start } from "./store";

start();
const overlay = new URLSearchParams(location.search).has("overlay");
if (overlay) document.documentElement.classList.add("overlay-window");
render(overlay ? <Overlay /> : <App />, document.getElementById("app")!);
