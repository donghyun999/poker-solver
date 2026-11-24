import { useState } from "react";
import { ConfigPanel } from "./components/ConfigPanel";
import { Viewer } from "./components/Viewer";
import "./App.css";

export default function App() {
  // 솔브/불러오기가 끝날 때마다 올려서 뷰어를 새로고침
  const [version, setVersion] = useState(0);

  return (
    <div className="app">
      <aside>
        <ConfigPanel onSolved={() => setVersion((v) => v + 1)} />
      </aside>
      <main>
        {version > 0 ? (
          <Viewer version={version} />
        ) : (
          <div className="empty-state">
            왼쪽에서 스팟을 설정하고 <b>솔브</b>를 누르세요.
            <br />
            저장해 둔 결과는 <b>불러오기</b>로 바로 열 수 있습니다.
          </div>
        )}
      </main>
    </div>
  );
}
