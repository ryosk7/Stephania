import {createRoot} from 'react-dom/client';
import './styles.css';
import {App} from './app';
import {initAppearance} from './theme';

initAppearance();
createRoot(document.getElementById('root')!).render(<App/>);
