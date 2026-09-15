import React, { useState } from 'react';
import ReactDOM from 'react-dom/client';
import { Sidebar } from '../components/Sidebar';
import { Thread } from '../components/Thread';
import { project, sessions, detailFor } from './fixtures';
import '../styles.css';
import './preview.css';

function Demo() {
  const [selected, setSelected] = useState(sessions[0]);
  const [notice, setNotice] = useState('Synthetic sessions · No local files or model connection');
  const unavailable = () => setNotice('Terminal, settings and write operations are available in the desktop app. This preview only displays synthetic sessions.');
  return <><div className="demo-banner"><strong>Pi Desktop / Preview</strong><span role="status">{notice}</span><a href="https://github.com/roshameow/pi-session-viewer">Source & setup ↗</a></div><div className="app demo-app">
    <Sidebar projects={[project]} sessions={sessions} selectedProject={project.key} selectedSessionPath={selected.path} loadingSessions={false} onSelectProject={()=>setSelected(sessions[0])} onSelectSession={setSelected} onRefresh={()=>setNotice('Synthetic preview refreshed.')} onOpenTerminal={unavailable} onDeleteSession={unavailable} onDetachFromRmux={unavailable} onKillRmuxSession={unavailable} onTransferToRemote={unavailable} onOpenConfig={unavailable} showConfig={false} finishedAt={{}} remoteHosts={[]} remoteHost={null} syncing={false} onSwitchSource={unavailable}/>
    <div className="main"><Thread key={selected.id} detail={detailFor(selected)} liveBlocks={[]} running={false} preview /></div>
  </div></>;
}
ReactDOM.createRoot(document.getElementById('root')!).render(<React.StrictMode><Demo /></React.StrictMode>);
