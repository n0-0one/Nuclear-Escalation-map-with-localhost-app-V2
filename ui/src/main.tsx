import { render } from "preact";
import "./style.css";
import { App } from "./App";
import { Overlay } from "./Overlay";
import { start } from "./store";

start();
const overlay = new URLSearchParams(location.search).has("overlay");
if (overlay) document.documentElement.classList.add("overlay-window");
render(overlay ? <Overlay /> : <App />, document.getElementById("app")!);
