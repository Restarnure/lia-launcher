import { lazy, Suspense } from "react";
import { setState, useStore } from "./lib/store";
import { CrashDialog, DropHandler, Sidebar, TitleBar, Toasts } from "./components/Shell";
import { NewInstanceModal } from "./components/NewInstance";
import { ConfirmHost } from "./components/ui";
import { Home } from "./pages/Home";
import { Instances } from "./pages/Instances";
import { InstanceView } from "./pages/InstanceView";
import { Browse } from "./pages/Browse";
import { AddAccount } from "./components/AddAccount";
import { Settings } from "./pages/Settings";

// 3D-перегляд скінів (three.js) — найважча частина; вантажиться лише на сторінці акаунтів.
const Accounts = lazy(() => import("./pages/Accounts").then((m) => ({ default: m.Accounts })));

function Page() {
  const route = useStore((s) => s.route);
  switch (route.page) {
    case "home":
      return <Home />;
    case "instances":
      return <Instances />;
    case "instance":
      return <InstanceView key={route.id} id={route.id} tab={route.tab} />;
    case "browse":
      return <Browse type={route.type} instanceId={route.instanceId} />;
    case "accounts":
      return (
        <Suspense fallback={<Splash />}>
          <Accounts />
        </Suspense>
      );
    case "settings":
      return <Settings />;
  }
}

export function App() {
  const ready = useStore((s) => s.ready);
  const lang = useStore((s) => s.settings?.language);
  const newInstance = useStore((s) => s.newInstanceOpen);
  const addAccount = useStore((s) => s.addAccountOpen);
  return (
    <div className="app" key={lang}>
      <TitleBar />
      <Sidebar />
      <main className="main">{ready ? <Page /> : <Splash />}</main>
      {newInstance && <NewInstanceModal />}
      {addAccount && <AddAccount onClose={() => setState({ addAccountOpen: false })} />}
      <CrashDialog />
      <ConfirmHost />
      <DropHandler />
      <Toasts />
    </div>
  );
}

function Splash() {
  return (
    <div className="splash">
      <div className="spinner" style={{ width: 28, height: 28 }} />
    </div>
  );
}
