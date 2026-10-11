/**
 * Публичный вход каркаса Office: страницы импортируют отсюда и только отсюда,
 * чтобы файлы каркаса можно было перетасовать, не трогая страницы.
 *
 * Стили подключаются здесь же: любой, кто взял компонент, получает и его вид,
 * и порядок правил в бандле задаётся одним импортом, а не порядком импортов
 * на каждой странице.
 */
import './office.css';

export { Icon } from './icons.js';
export type { IconName } from './icons.js';
export { LogoExcel, LogoWord } from './logos.js';
export { RibbonButton, RibbonGroup, RibbonSelect, RibbonTabs, RibbonTile, RibbonToggle } from './Ribbon.js';
export type { RibbonTab } from './Ribbon.js';
export { TitleBar } from './TitleBar.js';
export { StatusBar } from './StatusBar.js';
export { Ruler } from './Ruler.js';
export { FormulaBar } from './FormulaBar.js';
