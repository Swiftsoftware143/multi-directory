/* Directory SEO & Content Admin - Additional features for Multi-Directory admin panel */
(function(){
  if(typeof PGS==='undefined'){setTimeout(function(){location.reload();},2000);return;}
  PGS.push("services","locations","programmatic","topics","authors","seosettings","schemaconfig","seofallbacks","repurpose");
  TTL.services="Services"; TTL.locations="Locations"; TTL.programmatic="Landing Pages";
  TTL.topics="Content Calendar"; TTL.authors="Authors"; TTL.seosettings="SEO Settings";
  TTL.schemaconfig="Schema Config"; TTL.seofallbacks="SEO Fallbacks"; TTL.repurpose="Repurpose";
  ICO.services="🔧"; ICO.locations="📍"; ICO.programmatic="📄";
  ICO.topics="📅"; ICO.authors="✍️"; ICO.seosettings="⚙️";
  ICO.schemaconfig="🔖"; ICO.seofallbacks="📋"; ICO.repurpose="🔄";
  WT.services={i:"🔧",t:"Services",x:"Define services your directory covers."};
  WT.locations={i:"📍",t:"Locations",x:"Manage cities and neighborhoods. Bulk import via CSV."};
  WT.programmatic={i:"📄",t:"Landing Pages",x:"Auto-generate SEO landing pages from service+location combos."};
  WT.topics={i:"📅",t:"Content Calendar",x:"Plan blog posts from idea to publication."};
  WT.authors={i:"✍️",t:"Authors",x:"Manage author profiles for blog posts."};
  WT.seosettings={i:"⚙️",t:"SEO Settings",x:"Slug patterns, Maps API key, AI settings."};
  WT.schemaconfig={i:"🔖",t:"Schema Config",x:"Enable/disable schema markup types per directory."};
  WT.seofallbacks={i:"📋",t:"SEO Fallbacks",x:"Default title/description templates."};
  WT.repurpose={i:"🔄",t:"Repurpose",x:"Extract FAQ, email, social packs from posts."};
})();
var CURRENT_DIR=null;
function t(m,ty){toast(m,ty||"info");}

// ── SERVICES ──
async function loadServices() {
  var el=dom("pageContent"),pa=dom("pageActions");
  if(!CURRENT_DIR){el.innerHTML='<div class="empty-state"><div class="icon">📁</div><h3>Select a Directory</h3><p>Go to Directories and click "Manage" to scope features to a directory.</p></div>';pa.innerHTML='';return;}
  pa.innerHTML='<button class="btn btn-sm btn-primary" onclick="showServiceModal(null)">+ Add Service</button> <button class="btn btn-sm btn-secondary" onclick="importCsvServices()">📥 CSV Import</button>';
  el.innerHTML='<div style="text-align:center;padding:40px"><span class="spinner-teal"></span></div>';
  try{var data=await g("/api/v1/directories/"+CURRENT_DIR+"/services");if(!data.length){el.innerHTML='<div class="empty-state"><div class="icon">🔧</div><h3>No Services Defined</h3><p>Add services like "Plumber", "Electrician", "Dentist" that your directory covers.</p></div>';return;}
    var h='<div class="card-grid">';
    for(var i=0;i<data.length;i++){var s=data[i];
      h+='<div class="card"><div style="display:flex;align-items:center;justify-content:space-between;margin-bottom:6px"><h4>'+esc(s.name)+'</h4><span class="badge '+(s.is_active!==false?'active':'inactive')+'">'+(s.is_active!==false?'Active':'Inactive')+'</span></div><div><code style="font-size:.8rem">/'+esc(s.slug)+'</code></div><p style="font-size:.8rem;color:var(--text-secondary);margin-top:4px">'+esc(s.description||'')+'</p><div class="flex" style="margin-top:8px"><button class="btn btn-sm btn-secondary" onclick="editService(\''+s.id+'\')">Edit</button><button class="btn btn-sm btn-danger" onclick="deleteService(\''+s.id+'\')">Delete</button></div></div>';}
    el.innerHTML=h+'</div>';
  }catch(e){el.innerHTML='<div class="empty-state"><div class="icon">⚠️</div><h3>Error</h3><p>'+esc(e.message)+'</p></div>';}
}

function showServiceModal(id){
  var isEdit=!!id;var ov=modalOverlay('svcModal');
  ov.innerHTML='<div class="modal" style="width:480px"><h2>'+(isEdit?'Edit Service':'New Service')+'</h2><div class="form-group"><label>Name *</label><input id="svcName" placeholder="e.g. Plumber"></div><div class="form-group"><label>Slug</label><input id="svcSlug" placeholder="e.g. plumber"><div style="font-size:.75rem;color:var(--text-secondary)">Leave blank to auto-generate</div></div><div class="form-group"><label>Description</label><textarea id="svcDesc" rows="2" placeholder="Brief description"></textarea></div><div class="modal-actions"><button class="btn btn-secondary" onclick="closeModal(\'svcModal\')">Cancel</button><button class="btn btn-primary" onclick="saveService(\''+(isEdit?id:'')+'\')">Save</button></div></div>';
  docb(ov);
  if(isEdit)fetchService(id);
}

async function fetchService(id){
  try{var data=await g("/api/v1/directories/"+CURRENT_DIR+"/services");
    for(var i=0;i<data.length;i++){if(data[i].id===id){var s=data[i];domById('svcName').value=s.name;domById('svcSlug').value=s.slug;domById('svcDesc').value=s.description||'';return;}}
  }catch(e){t(e.message,'error');}
}

async function saveService(id){
  var name=domById('svcName').value.trim(),slug=domById('svcSlug').value.trim(),desc=domById('svcDesc').value.trim();
  if(!name){t('Name is required','error');return;}
  try{if(id){await pu("/api/v1/directories/"+CURRENT_DIR+"/services/"+id,{name:name,slug:slug||null,description:desc||null});}
    else{await po("/api/v1/directories/"+CURRENT_DIR+"/services",{name:name,slug:slug||null,description:desc||null});}
    t('Service saved!');closeModal('svcModal');loadServices();
  }catch(e){t(e.message,'error');}
}
function editService(id){showServiceModal(id);}
async function deleteService(id){if(!confirm('Delete this service?'))return;try{await dl("/api/v1/directories/"+CURRENT_DIR+"/services/"+id);t('Deleted');loadServices();}catch(e){t(e.message,'error');}}

// ── CSV Import: Services ──
function importCsvServices(){
  var csv=prompt("Paste CSV rows (one per line):\nName, Slug (optional), Description (optional)\n\nExample:\nPlumber,plumber,Specializes in residential\nElectrician,electrician,Commercial & residential");
  if(!csv)return;
  var rows=csv.trim().split('\n').map(function(line){return line.split(',').map(function(c){return c.trim();});});
  if(rows.length<1){t('No rows found','error');return;}
  po("/api/v1/directories/"+CURRENT_DIR+"/services/import",{rows:rows})
    .then(function(r){t('Imported: '+r.created+' created, '+r.skipped+' skipped','success');loadServices();})
    .catch(function(e){t(e.message,'error');});
}
function importCsvLocations(){
  var csv=prompt("Paste CSV rows (one per line):\nName, Slug (optional), State (optional), Region (optional)\n\nExample:\nMiami,miami,Florida,South Florida\nFort Lauderdale,fort-lauderdale,Florida,South Florida");
  if(!csv)return;
  var rows=csv.trim().split('\n').map(function(line){return line.split(',').map(function(c){return c.trim();});});
  if(rows.length<1){t('No rows found','error');return;}
  po("/api/v1/directories/"+CURRENT_DIR+"/locations/import",{rows:rows})
    .then(function(r){t('Imported: '+r.created+' created, '+r.skipped+' skipped','success');loadLocations();})
    .catch(function(e){t(e.message,'error');});
}

// ── LOCATIONS ──
async function loadLocations(){
  var el=dom('pageContent'),pa=dom('pageActions');
  if(!CURRENT_DIR){el.innerHTML='<div class="empty-state"><div class="icon">📁</div><h3>Select a Directory</h3></div>';pa.innerHTML='';return;}
  pa.innerHTML='<button class="btn btn-sm btn-primary" onclick="showLocationModal(null)">+ Add Location</button> <button class="btn btn-sm btn-secondary" onclick="importCsvLocations()">📥 CSV Import</button>';
  el.innerHTML='<div style="text-align:center;padding:40px"><span class="spinner-teal"></span></div>';
  try{var data=await g("/api/v1/directories/"+CURRENT_DIR+"/locations");if(!data.length){el.innerHTML='<div class="empty-state"><div class="icon">📍</div><h3>No Locations Added</h3><p>Add cities, neighborhoods, or regions your directory covers.</p></div>';return;}
    var h='<div class="card-grid">';
    for(var i=0;i<data.length;i++){var l=data[i];
      h+='<div class="card"><div style="display:flex;align-items:center;justify-content:space-between;margin-bottom:6px"><h4>'+esc(l.name)+'</h4><span class="badge '+(l.is_active!==false?'active':'inactive')+'">'+(l.is_active!==false?'Active':'Inactive')+'</span></div><div><code style="font-size:.8rem">/'+esc(l.slug)+'</code>'+(l.state?' <span style="font-size:.8rem;color:var(--text-secondary)">'+esc(l.state)+'</span>':'')+(l.region?' <span style="font-size:.8rem;color:var(--text-secondary)">· '+esc(l.region)+'</span>':'')+'</div><div class="flex" style="margin-top:8px"><button class="btn btn-sm btn-secondary" onclick="editLocation(\''+l.id+'\')">Edit</button><button class="btn btn-sm btn-danger" onclick="deleteLocation(\''+l.id+'\')">Delete</button></div></div>';}
    el.innerHTML=h+'</div>';
  }catch(e){el.innerHTML='<div class="empty-state"><div class="icon">⚠️</div><h3>Error</h3><p>'+esc(e.message)+'</p></div>';}
}

function showLocationModal(id){
  var isEdit=!!id;var ov=modalOverlay('locModal');
  ov.innerHTML='<div class="modal" style="width:480px"><h2>'+(isEdit?'Edit Location':'New Location')+'</h2><div class="form-group"><label>Name *</label><input id="locName" placeholder="e.g. Miami"></div><div class="form-group"><label>Slug</label><input id="locSlug" placeholder="e.g. miami"></div><div class="form-group"><label>State/Province</label><input id="locState" placeholder="e.g. Florida"></div><div class="form-group"><label>Region</label><input id="locRegion" placeholder="e.g. South Florida"></div><div class="modal-actions"><button class="btn btn-secondary" onclick="closeModal(\'locModal\')">Cancel</button><button class="btn btn-primary" onclick="saveLocation(\''+(isEdit?id:'')+'\')">Save</button></div></div>';
  docb(ov);if(isEdit)fetchLocation(id);
}

async function fetchLocation(id){
  try{var data=await g("/api/v1/directories/"+CURRENT_DIR+"/locations");for(var i=0;i<data.length;i++){if(data[i].id===id){var l=data[i];domById('locName').value=l.name;domById('locSlug').value=l.slug;domById('locState').value=l.state||'';domById('locRegion').value=l.region||'';return;}}
  }catch(e){t(e.message,'error');}
}

async function saveLocation(id){
  var name=domById('locName').value.trim(),slug=domById('locSlug').value.trim(),state=domById('locState').value.trim(),region=domById('locRegion').value.trim();
  if(!name){t('Name is required','error');return;}
  var body={name:name,slug:slug||null,state:state||null,region:region||null};
  try{if(id){await pu("/api/v1/directories/"+CURRENT_DIR+"/locations/"+id,body);}else{await po("/api/v1/directories/"+CURRENT_DIR+"/locations",body);}
    t('Location saved!');closeModal('locModal');loadLocations();
  }catch(e){t(e.message,'error');}
}
function editLocation(id){showLocationModal(id);}
async function deleteLocation(id){if(!confirm('Delete this location?'))return;try{await dl("/api/v1/directories/"+CURRENT_DIR+"/locations/"+id);t('Deleted');loadLocations();}catch(e){t(e.message,'error');}}

// ── PROGRAMMATIC PAGES ──
async function loadProgrammatic(){
  var el=dom('pageContent'),pa=dom('pageActions');
  if(!CURRENT_DIR){el.innerHTML='<div class="empty-state"><div class="icon">📁</div><h3>Select a Directory</h3></div>';pa.innerHTML='';return;}
  pa.innerHTML='<button class="btn btn-sm btn-primary" onclick="showGeneratePages()">⚡ Generate Pages</button>';
  el.innerHTML='<div style="text-align:center;padding:40px"><span class="spinner-teal"></span></div>';
  try{var data=await g("/api/v1/directories/"+CURRENT_DIR+"/programmatic-pages");if(!data.length){el.innerHTML='<div class="empty-state"><div class="icon">📄</div><h3>No Landing Pages Yet</h3><p>Use "Generate Pages" to create SEO pages from Service x Location combinations.</p></div>';return;}
    var h='<div class="table-wrap"><table><thead><tr><th>Title</th><th>Service</th><th>Location</th><th>Slug</th><th>Status</th><th>Actions</th></tr></thead><tbody>';
    for(var i=0;i<data.length;i++){var p=data[i];
      h+='<tr><td>'+esc(p.title||'[No Title]')+'</td><td>'+esc(p.service_name||'-')+'</td><td>'+esc(p.location_name||'-')+'</td><td><code style="font-size:.75rem">/'+esc(p.slug)+'</code></td><td><span class="badge '+(p.status==='published'?'active':p.status==='draft'?'inactive':'')+'">'+esc(p.status||'draft')+'</span></td><td><button class="btn btn-sm btn-secondary" onclick="editProgrammaticPage(\''+p.id+'\')">Edit</button></td></tr>';}
    el.innerHTML=h+'</tbody></table></div>';
  }catch(e){el.innerHTML='<div class="empty-state"><div class="icon">⚠️</div><h3>Error</h3><p>'+esc(e.message)+'</p></div>';}
}

function showGeneratePages(){
  var ov=modalOverlay('genModal');
  Promise.all([g("/api/v1/directories/"+CURRENT_DIR+"/services"),g("/api/v1/directories/"+CURRENT_DIR+"/locations")]).then(function(results){
    var services=results[0],locations=results[1];
    var svcHtml='',locHtml='';
    for(var i=0;i<services.length;i++){svcHtml+='<label style="display:flex;align-items:center;gap:6px;padding:3px 0;font-size:.85rem"><input type="checkbox" class="gen-svc" value="'+services[i].id+'">'+esc(services[i].name)+'</label>';}
    for(var i=0;i<locations.length;i++){locHtml+='<label style="display:flex;align-items:center;gap:6px;padding:3px 0;font-size:.85rem"><input type="checkbox" class="gen-loc" value="'+locations[i].id+'">'+esc(locations[i].name)+'</label>';}
    if(!svcHtml)svcHtml='<p style="font-size:.8rem;color:var(--text-secondary)">No services. Add services first.</p>';
    if(!locHtml)locHtml='<p style="font-size:.8rem;color:var(--text-secondary)">No locations. Add locations first.</p>';
    ov.innerHTML='<div class="modal" style="width:600px"><h2>⚡ Generate Landing Pages</h2><p style="font-size:.85rem;color:var(--text-secondary);margin-bottom:16px">Select services and locations to generate SEO landing pages.</p><div style="display:grid;grid-template-columns:1fr 1fr;gap:16px"><div><h4 style="font-size:.85rem;margin-bottom:8px">Services</h4>'+svcHtml+'</div><div><h4 style="font-size:.85rem;margin-bottom:8px">Locations</h4>'+locHtml+'</div></div><div style="margin-top:16px"><button class="btn btn-sm btn-secondary" onclick="selectAllGen(\'gen-svc\')">Select All Services</button> <button class="btn btn-sm btn-secondary" onclick="selectAllGen(\'gen-loc\')">Select All Locations</button></div><div class="modal-actions"><button class="btn btn-secondary" onclick="closeModal(\'genModal\')">Cancel</button><button class="btn btn-primary" onclick="doGeneratePages()">⚡ Generate Pages</button></div></div>';
  }).catch(function(e){t(e.message,'error');closeModal('genModal');});
}

function selectAllGen(cls){var cb=document.querySelectorAll('.'+cls);for(var i=0;i<cb.length;i++)cb[i].checked=true;}

function doGeneratePages(){
  var svcIds=[],locIds=[];
  var svcEl=document.querySelectorAll('.gen-svc:checked'),locEl=document.querySelectorAll('.gen-loc:checked');
  for(var i=0;i<svcEl.length;i++)svcIds.push(svcEl[i].value);
  for(var i=0;i<locEl.length;i++)locIds.push(locEl[i].value);
  if(!svcIds.length||!locIds.length){t('Select at least one service and one location','error');return;}
  po("/api/v1/directories/"+CURRENT_DIR+"/programmatic-pages/generate",{service_ids:svcIds,location_ids:locIds})
    .then(function(r){t('Generated '+r.created+' pages ('+r.skipped+' existed)','success');closeModal('genModal');loadProgrammatic();})
    .catch(function(e){t(e.message,'error');});
}

async function editProgrammaticPage(id){
  try{var p=await g("/api/v1/directories/"+CURRENT_DIR+"/programmatic-pages/"+id);
    var ov=modalOverlay('editPageModal');
    ov.innerHTML='<div class="modal" style="width:600px"><h2>Edit Landing Page</h2><div class="form-group"><label>Title</label><input id="ppTitle" value="'+esc(p.title||'')+'"></div><div class="form-group"><label>Meta Title</label><input id="ppMetaTitle" value="'+esc(p.meta_title||'')+'"></div><div class="form-group"><label>Meta Description</label><input id="ppMetaDesc" value="'+esc(p.meta_description||'')+'"></div><div class="form-group"><label>Status</label><select id="ppStatus"><option value="draft"'+(p.status==='draft'?' selected':'')+'">Draft</option><option value="published"'+(p.status==='published'?' selected':'')+'">Published</option></select></div><div class="modal-actions"><button class="btn btn-secondary" onclick="closeModal(\'editPageModal\')">Cancel</button><button class="btn btn-primary" onclick="saveProgrammaticPage(\''+id+'\')">Save</button></div></div>';
    docb(ov);
  }catch(e){t(e.message,'error');}
}

async function saveProgrammaticPage(id){
  var title=domById('ppTitle').value.trim(),mt=domById('ppMetaTitle').value.trim(),md=domById('ppMetaDesc').value.trim(),st=domById('ppStatus').value;
  try{await pu("/api/v1/directories/"+CURRENT_DIR+"/programmatic-pages/"+id,{title:title||null,meta_title:mt||null,meta_description:md||null,status:st});
    t('Page updated!');closeModal('editPageModal');loadProgrammatic();
  }catch(e){t(e.message,'error');}
}

// ── TOPICS / CONTENT CALENDAR ──
async function loadTopics(){
  var el=dom('pageContent'),pa=dom('pageActions');
  if(!CURRENT_DIR){el.innerHTML='<div class="empty-state"><div class="icon">📁</div><h3>Select a Directory</h3></div>';pa.innerHTML='';return;}
  pa.innerHTML='<button class="btn btn-sm btn-primary" onclick="showTopicModal(null)">+ New Topic</button> <button class="btn btn-sm btn-secondary" onclick="suggestTopics()">💡 Suggest Topics</button>';
  el.innerHTML='<div style="text-align:center;padding:40px"><span class="spinner-teal"></span></div>';
  try{var data=await g("/api/v1/directories/"+CURRENT_DIR+"/topics");if(!data.length){el.innerHTML='<div class="empty-state"><div class="icon">📅</div><h3>No Topics Yet</h3><p>Create content topics or click "Suggest Topics" to generate ideas.</p></div>';return;}
    var h='<div class="card-grid">';
    var scMap={suggested:'#f59e0b',scheduled:'#3b82f6',in_progress:'#8b5cf6',in_review:'#ec4899',published:'#22c55e'};
    for(var i=0;i<data.length;i++){var tp=data[i];var sc=scMap[tp.status]||'#94a3b8';
      h+='<div class="card" style="border-left:4px solid '+sc+'"><div style="display:flex;align-items:center;justify-content:space-between;margin-bottom:4px"><span class="badge" style="background:'+sc+';color:#fff">'+esc(tp.status||'suggested')+'</span>'+(tp.scheduled_date?'<span style="font-size:.75rem;color:var(--text-secondary)">📅 '+tp.scheduled_date.substring(0,10)+'</span>':'')+'</div><h4 style="font-size:.9rem;margin-bottom:4px">'+esc(tp.title)+'</h4><div style="font-size:.75rem;color:var(--text-secondary)">'+(tp.service_name?esc(tp.service_name)+' · ':'')+(tp.location_name?esc(tp.location_name)+' · ':'')+(tp.target_keyword?'<code>'+esc(tp.target_keyword)+'</code>':'')+(tp.word_count_target?' · '+tp.word_count_target+' words':'')+'</div><div class="flex" style="margin-top:8px"><button class="btn btn-sm btn-secondary" onclick="editTopic(\''+tp.id+'\')">Edit</button><button class="btn btn-sm btn-danger" onclick="deleteTopic(\''+tp.id+'\')">Delete</button></div></div>';}
    el.innerHTML=h+'</div>';
  }catch(e){el.innerHTML='<div class="empty-state"><div class="icon">⚠️</div><h3>Error</h3><p>'+esc(e.message)+'</p></div>';}
}

function showTopicModal(id){
  var isEdit=!!id;var ov=modalOverlay('topicModal');
  ov.innerHTML='<div class="modal" style="width:520px"><h2>'+(isEdit?'Edit Topic':'New Content Topic')+'</h2><div class="form-group"><label>Title *</label><input id="tpTitle" placeholder="e.g. How to Choose a Plumber in Miami"></div><div class="form-group" style="display:grid;grid-template-columns:1fr 1fr;gap:8px"><div><label>Target Keyword</label><input id="tpKeyword" placeholder="e.g. plumber miami"></div><div><label>Word Count</label><input id="tpWords" type="number" value="1000" min="300" max="5000"></div></div><div class="form-group" style="display:grid;grid-template-columns:1fr 1fr;gap:8px"><div><label>Status</label><select id="tpStatus"><option value="suggested">Suggested</option><option value="scheduled">Scheduled</option><option value="in_progress">In Progress</option><option value="in_review">In Review</option><option value="published">Published</option></select></div><div><label>Scheduled Date</label><input id="tpDate" type="date"></div></div><div class="modal-actions"><button class="btn btn-secondary" onclick="closeModal(\'topicModal\')">Cancel</button><button class="btn btn-primary" onclick="saveTopic(\''+(isEdit?id:'')+'\')">Save</button></div></div>';
  docb(ov);if(isEdit)fetchTopic(id);
}

async function fetchTopic(id){
  try{var data=await g("/api/v1/directories/"+CURRENT_DIR+"/topics");for(var i=0;i<data.length;i++){if(data[i].id===id){var tp=data[i];domById('tpTitle').value=tp.title;domById('tpKeyword').value=tp.target_keyword||'';domById('tpWords').value=tp.word_count_target||1000;if(tp.status)domById('tpStatus').value=tp.status;if(tp.scheduled_date)domById('tpDate').value=tp.scheduled_date.substring(0,10);return;}}
  }catch(e){t(e.message,'error');}
}

async function saveTopic(id){
  var title=domById('tpTitle').value.trim();if(!title){t('Title is required','error');return;}
  var body={title:title,target_keyword:domById('tpKeyword').value.trim()||null,word_count_target:parseInt(domById('tpWords').value)||1000,status:domById('tpStatus').value,scheduled_date:domById('tpDate').value?new Date(domById('tpDate').value).toISOString():null};
  try{if(id){await pu("/api/v1/directories/"+CURRENT_DIR+"/topics/"+id,body);}else{await po("/api/v1/directories/"+CURRENT_DIR+"/topics",body);}
    t('Topic saved!');closeModal('topicModal');loadTopics();
  }catch(e){t(e.message,'error');}
}
function editTopic(id){showTopicModal(id);}
async function deleteTopic(id){if(!confirm('Delete this topic?'))return;try{await dl("/api/v1/directories/"+CURRENT_DIR+"/topics/"+id);t('Deleted');loadTopics();}catch(e){t(e.message,'error');}}

async function suggestTopics(){
  try{var suggestions=await g("/api/v1/directories/"+CURRENT_DIR+"/topics/suggestions");if(!suggestions.length){t('No suggestions. Add services and locations first.','error');return;}
    var ov=modalOverlay('suggestModal');
    var html='<div class="modal" style="width:640px;max-height:80vh;overflow-y:auto"><h2>💡 Suggested Topics</h2><p style="font-size:.85rem;color:var(--text-secondary);margin-bottom:12px">Click "Add" to add to calendar:</p>';
    for(var i=0;i<suggestions.length;i++){var sg=suggestions[i];
      html+='<div style="border:1px solid var(--border);border-radius:8px;padding:10px;margin-bottom:8px"><div style="display:flex;justify-content:space-between;align-items:flex-start"><div><strong style="font-size:.9rem">'+esc(sg.title)+'</strong><br><span style="font-size:.75rem;color:var(--text-secondary)">'+esc(sg.service_name)+' · '+esc(sg.location_name)+' · <code>'+esc(sg.target_keyword)+'</code></span></div><button class="btn btn-sm btn-primary" onclick="addSuggestedTopic(\''+esc(sg.title).replace(/'/g,"\\'")+'\',\''+esc(sg.target_keyword)+'\')">+ Add</button></div></div>';}
    html+='<div class="modal-actions"><button class="btn btn-secondary" onclick="closeModal(\'suggestModal\')">Close</button></div></div>';
    ov.innerHTML=html;docb(ov);
  }catch(e){t(e.message,'error');}
}

async function addSuggestedTopic(title,kw){
  try{await po("/api/v1/directories/"+CURRENT_DIR+"/topics",{title:title,target_keyword:kw||null,status:'suggested'});t('Topic added!','success');}catch(e){t(e.message,'error');}
}

// ── AUTHORS ──
async function loadAuthors(){
  var el=dom('pageContent'),pa=dom('pageActions');
  if(!CURRENT_DIR){el.innerHTML='<div class="empty-state"><div class="icon">📁</div><h3>Select a Directory</h3></div>';pa.innerHTML='';return;}
  pa.innerHTML='<button class="btn btn-sm btn-primary" onclick="showAuthorModal(null)">+ New Author</button>';
  el.innerHTML='<div style="text-align:center;padding:40px"><span class="spinner-teal"></span></div>';
  try{var data=await g("/api/v1/directories/"+CURRENT_DIR+"/authors");if(!data.length){el.innerHTML='<div class="empty-state"><div class="icon">✍️</div><h3>No Authors Yet</h3><p>Create author profiles for blog posts.</p></div>';return;}
    var h='<div class="card-grid">';
    for(var i=0;i<data.length;i++){var a=data[i];
      h+='<div class="card"><div style="display:flex;align-items:center;gap:12px"><div style="width:40px;height:40px;border-radius:50%;background:var(--teal);color:#fff;display:flex;align-items:center;justify-content:center;font-size:1.1rem">'+(a.avatar_url?'<img src="'+esc(a.avatar_url)+'" style="width:40px;height:40px;border-radius:50%;object-fit:cover">':a.name.charAt(0).toUpperCase())+'</div><div><h4 style="font-size:.9rem">'+esc(a.name)+'</h4><span class="badge '+(a.is_active!==false?'active':'inactive')+'">'+esc(a.role||'author')+'</span></div></div><div style="font-size:.8rem;color:var(--text-secondary);margin-top:8px">'+(a.bio?esc(a.bio.substring(0,100)):'')+'</div><div class="flex" style="margin-top:8px"><button class="btn btn-sm btn-secondary" onclick="editAuthor(\''+a.id+'\')">Edit</button><button class="btn btn-sm btn-danger" onclick="deleteAuthor(\''+a.id+'\')">Delete</button></div></div>';}
    el.innerHTML=h+'</div>';
  }catch(e){el.innerHTML='<div class="empty-state"><div class="icon">⚠️</div><h3>Error</h3><p>'+esc(e.message)+'</p></div>';}
}

function showAuthorModal(id){
  var isEdit=!!id;var ov=modalOverlay('authorModal');
  ov.innerHTML='<div class="modal" style="width:520px"><h2>'+(isEdit?'Edit Author':'New Author')+'</h2><div class="form-group"><label>Name *</label><input id="authName" placeholder="e.g. John Smith"></div><div class="form-group"><label>Email</label><input id="authEmail" type="email" placeholder="author@example.com"></div><div class="form-group"><label>Role</label><select id="authRole"><option value="author">Author</option><option value="editor">Editor</option><option value="contributor">Contributor</option></select></div><div class="form-group"><label>Bio</label><textarea id="authBio" rows="2" placeholder="Short bio"></textarea></div><div class="form-group"><label>Avatar URL</label><input id="authAvatar" placeholder="https://example.com/avatar.jpg"></div><div class="form-group"><label>Slug (URL handle)</label><input id="authSlug" placeholder="john-smith"></div><div class="modal-actions"><button class="btn btn-secondary" onclick="closeModal(\'authorModal\')">Cancel</button><button class="btn btn-primary" onclick="saveAuthor(\''+(isEdit?id:'')+'\')">Save</button></div></div>';
  docb(ov);if(isEdit)fetchAuthor(id);
}

async function fetchAuthor(id){
  try  async function fetchAuthor(id){
  try{var data=await g("/api/v1/directories/"+CURRENT_DIR+"/authors");for(var i=0;i<data.length;i++){if(data[i].id===id){var a=data[i];domById('authName').value=a.name;domById('authEmail').value=a.email||'';domById('authRole').value=a.role||'author';domById('authBio').value=a.bio||'';domById('authAvatar').value=a.avatar_url||'';domById('authSlug').value=a.slug||'';return;}}
  }catch(e){t(e.message,'error');}
}

async function saveAuthor(id){
  var name=domById('authName').value.trim();if(!name){t('Name is required','error');return;}
  var body={name:name,email:domById('authEmail').value.trim()||null,role:domById('authRole').value,bio:domById('authBio').value.trim()||null,avatar_url:domById('authAvatar').value.trim()||null,slug:domById('authSlug').value.trim()||null};
  try{if(id){await pu("/api/v1/directories/"+CURRENT_DIR+"/authors/"+id,body);}else{await po("/api/v1/directories/"+CURRENT_DIR+"/authors",body);}
    t('Author saved!');closeModal('authorModal');loadAuthors();
  }catch(e){t(e.message,'error');}
}
function editAuthor(id){showAuthorModal(id);}
async function deleteAuthor(id){if(!confirm('Delete this author?'))return;try{await dl("/api/v1/directories/"+CURRENT_DIR+"/authors/"+id);t('Deleted');loadAuthors();}catch(e){t(e.message,'error');}}

// ── SEO SETTINGS ──
async function loadSeosettings(){
  var el=dom('pageContent'),pa=dom('pageActions');
  if(!CURRENT_DIR){el.innerHTML='<div class="empty-state"><div class="icon">📁</div><h3>Select a Directory</h3></div>';pa.innerHTML='';return;}
  pa.innerHTML='<button class="btn btn-primary btn-sm" onclick="saveSeoSettings()">💾 Save Settings</button>';
  el.innerHTML='<div style="text-align:center;padding:40px"><span class="spinner-teal"></span></div>';
  try{
    var dir=await g("/api/v1/directories/"+CURRENT_DIR);
    var cfg={};
    try{cfg=await g("/api/v1/directories/"+CURRENT_DIR+"/seo-settings");}catch(e2){}
    el.innerHTML='<div class="card" style="max-width:640px"><h2 style="margin-bottom:16px">SEO Settings</h2>'+
      '<div class="form-group"><label>Page Slug Pattern</label><input id="seoSlugPattern" value="'+esc(cfg.page_slug_pattern||dir.page_slug_pattern||'/{service}/{city}')+'"><div style="font-size:.75rem;color:var(--text-secondary)">Uses {service}, {city}, {state}, {region}</div></div>'+
      '<div class="form-group"><label>Google Maps API Key</label><input id="seoMapsKey" value="'+esc(cfg.google_maps_api_key||'')+'" placeholder="AIza..."><div style="font-size:.75rem;color:var(--text-secondary)">Map embed hidden if no key configured</div></div>'+
      '<div class="form-group"><label>AI Content Model</label><select id="seoAiModel"><option value="claude-3-haiku" '+(cfg.ai_model==='claude-3-haiku'?'selected':'')+'>Claude 3 Haiku</option><option value="gpt-4o-mini" '+(cfg.ai_model==='gpt-4o-mini'?'selected':'')+'>GPT-4o Mini</option><option value="gpt-4o" '+(cfg.ai_model==='gpt-4o'?'selected':'')+'>GPT-4o</option></select></div>'+
      '<div class="form-group"><label>AI API Key</label><input id="seoAiKey" value="'+esc(cfg.ai_api_key||'')+'" placeholder="sk-..." type="password"></div>'+
      '<div class="form-group"><label>AI Prompt for Page Content</label><textarea id="seoAiPrompt" rows="3" placeholder="Write engaging content about {service} in {city}...">'+esc(cfg.ai_prompt||'')+'</textarea></div>'+
      '<div class="form-group"><div style="display:flex;gap:12px;align-items:center"><label style="display:flex;align-items:center;gap:4px"><input type="checkbox" id="seoAutoPublish" '+(cfg.auto_publish?'checked':'')+'> Auto-publish generated pages</label><label style="display:flex;align-items:center;gap:4px"><input type="checkbox" id="seoEnableSitemap" '+(cfg.enable_sitemap!==false?'checked':'')+'> Enable XML Sitemap</label></div></div>'+
      '<div class="form-group"><label>Internal Linking Min Keyword Matches</label><input id="seoLinkMin" type="number" value="'+(cfg.internal_link_min_matches||2)+'" min="1" max="10" style="width:80px"></div>'+
      '</div>';
    // Store for save
    dom('_seoCfg',cfg);
  }catch(e){el.innerHTML='<div class="empty-state"><div class="icon">⚠️</div><h3>Error</h3><p>'+esc(e.message)+'</p></div>';}
}

async function saveSeoSettings(){
  var body={
    page_slug_pattern:domById('seoSlugPattern').value.trim(),
    google_maps_api_key:domById('seoMapsKey').value.trim()||null,
    ai_model:domById('seoAiModel').value,
    ai_api_key:domById('seoAiKey').value.trim()||null,
    ai_prompt:domById('seoAiPrompt').value.trim()||null,
    auto_publish:domById('seoAutoPublish').checked,
    enable_sitemap:domById('seoEnableSitemap').checked,
    internal_link_min_matches:parseInt(domById('seoLinkMin').value)||2
  };
  try{await pu("/api/v1/directories/"+CURRENT_DIR+"/seo-settings",body);t('SEO settings saved!');loadSeosettings();}catch(e){t(e.message,'error');}
}

// ── SCHEMA CONFIG ──
async function loadSchemaconfig(){
  var el=dom('pageContent'),pa=dom('pageActions');
  if(!CURRENT_DIR){el.innerHTML='<div class="empty-state"><div class="icon">📁</div><h3>Select a Directory</h3></div>';pa.innerHTML='';return;}
  pa.innerHTML='<button class="btn btn-primary btn-sm" onclick="saveSchemaConfig()">💾 Save Schema Config</button>';
  el.innerHTML='<div style="text-align:center;padding:40px"><span class="spinner-teal"></span></div>';
  try{
    var configs=[];
    try{configs=await g("/api/v1/directories/"+CURRENT_DIR+"/schema-config");}catch(e2){}
    var schemaTypes=['LocalBusiness','FAQPage','Article','Product','Event','Service','Review','BreadcrumbList','ItemList','Organization'];
    var html='<div class="card" style="max-width:700px"><h2 style="margin-bottom:16px">Schema Markup Configuration</h2><p style="font-size:.85rem;color:var(--text-secondary);margin-bottom:16px">Enable or disable schema markup types for this directory. Each type can be toggled without code deployment.</p><table><thead><tr><th>Schema Type</th><th>Enabled</th><th>Settings</th><th>Actions</th></tr></thead><tbody>';
    for(var i=0;i<schemaTypes.length;i++){
      var st=schemaTypes[i];
      var found=null;
      for(var j=0;j<configs.length;j++){if(configs[j].schema_type===st){found=configs[j];break;}}
      var enabled=found?found.is_enabled!==false:true;
      html+='<tr><td><strong>'+st+'</strong></td><td><input type="checkbox" class="sc-toggle" data-type="'+st+'" '+(enabled?'checked':'')+'></td><td><input class="sc-settings" data-type="'+st+'" placeholder="JSON overrides" value="'+(found&&found.settings?esc(JSON.stringify(found.settings)):'')+'" style="width:200px;font-size:.75rem"></td><td><button class="btn btn-sm btn-secondary" onclick="editSchemaConfig(\''+st+'\')">Edit</button></td></tr>';
    }
    html+='</tbody></table></div>';
    el.innerHTML=html;
  }catch(e){el.innerHTML='<div class="empty-state"><div class="icon">⚠️</div><h3>Error</h3><p>'+esc(e.message)+'</p></div>';}
}

async function saveSchemaConfig(){
  var toggles=document.querySelectorAll('.sc-toggle');
  for(var i=0;i<toggles.length;i++){
    var type=toggles[i].getAttribute('data-type');
    var enabled=toggles[i].checked;
    var settingsEl=document.querySelector('.sc-settings[data-type="'+type+'"]');
    var settings=null;
    try{if(settingsEl&&settingsEl.value.trim())settings=JSON.parse(settingsEl.value.trim());}catch(e){}
    try{await pu("/api/v1/directories/"+CURRENT_DIR+"/schema-config/"+type,{is_enabled:enabled,settings:settings});}catch(e){t('Error saving '+type+': '+e.message,'error');}
  }
  t('Schema config saved!','success');
}

function editSchemaConfig(type){
  var ov=modalOverlay('scEditModal');
  try{var configs=JSON.parse(document.querySelector('.sc-settings[data-type="'+type+'"]').value)||{};}catch(e){var configs={};}
  ov.innerHTML='<div class="modal" style="width:560px"><h2>🔖 '+type+' Schema Settings</h2><div class="form-group"><label>Custom Properties (JSON)</label><textarea id="scCustomProps" rows="10" style="font-family:monospace;font-size:.8rem">'+esc(JSON.stringify(configs,null,2))+'</textarea></div><div class="modal-actions"><button class="btn btn-secondary" onclick="closeModal(\'scEditModal\')">Cancel</button><button class="btn btn-primary" onclick="saveSchemaCustom(\''+type+'\')">Save</button></div></div>';
  docb(ov);
}

async function saveSchemaCustom(type){
  try{var props=JSON.parse(domById('scCustomProps').value);}catch(e){t('Invalid JSON','error');return;}
  var el=document.querySelector('.sc-settings[data-type="'+type+'"]');
  if(el)el.value=JSON.stringify(props);
  await saveSchemaConfig();
  closeModal('scEditModal');
  t('Schema config updated!','success');
}

// ── SEO FALLBACKS ──
async function loadSeofallbacks(){
  var el=dom('pageContent'),pa=dom('pageActions');
  if(!CURRENT_DIR){el.innerHTML='<div class="empty-state"><div class="icon">📁</div><h3>Select a Directory</h3></div>';pa.innerHTML='';return;}
  pa.innerHTML='<button class="btn btn-primary btn-sm" onclick="saveSeoFallbacks()">💾 Save Fallbacks</button>';
  el.innerHTML='<div style="text-align:center;padding:40px"><span class="spinner-teal"></span></div>';
  try{
    var fallbacks={};
    try{fallbacks=await g("/api/v1/directories/"+CURRENT_DIR+"/seo-fallbacks");}catch(e2){}
    el.innerHTML='<div class="card" style="max-width:640px"><h2 style="margin-bottom:16px">SEO Fallback Templates</h2><p style="font-size:.85rem;color:var(--text-secondary);margin-bottom:16px">These templates are used when a page has no custom meta. Use variables: {service}, {city}, {state}, {region}, {site_name}, {meta_description}</p>'+
      '<div class="form-group"><label>Title Template (Programmatic Pages)</label><input id="fbTitle" value="'+esc(fallbacks.programmatic_title||'')+'" placeholder="e.g. Best {service} in {city} - {site_name}"></div>'+
      '<div class="form-group"><label>Description Template (Programmatic Pages)</label><textarea id="fbDesc" rows="2" placeholder="e.g. Find the best {service} in {city}. Read reviews and compare top-rated {service} near you.">'+esc(fallbacks.programmatic_description||'')+'</textarea></div>'+
      '<div class="form-group"><label>H1 Template</label><input id="fbH1" value="'+esc(fallbacks.programmatic_h1||'')+'" placeholder="e.g. Top {service} in {city}"></div>'+
      '<div class="form-group"><label>Blog Title Template</label><input id="fbBlogTitle" value="'+esc(fallbacks.blog_title||'')+'" placeholder="e.g. {title} - {site_name}"></div>'+
      '<div class="form-group"><label>Blog Description Template</label><textarea id="fbBlogDesc" rows="2">'+esc(fallbacks.blog_description||'')+'</textarea></div>'+
      '</div>';
  }catch(e){el.innerHTML='<div class="empty-state"><div class="icon">⚠️</div><h3>Error</h3><p>'+esc(e.message)+'</p></div>';}
}

async function saveSeoFallbacks(){
  var body={
    programmatic_title:domById('fbTitle').value.trim()||null,
    programmatic_description:domById('fbDesc').value.trim()||null,
    programmatic_h1:domById('fbH1').value.trim()||null,
    blog_title:domById('fbBlogTitle').value.trim()||null,
    blog_description:domById('fbBlogDesc').value.trim()||null
  };
  try{await pu("/api/v1/directories/"+CURRENT_DIR+"/seo-fallbacks",body);t('Fallbacks saved!');loadSeofallbacks();}catch(e){t(e.message,'error');}
}

// ── REPURPOSE ──
async function loadRepurpose(){
  var el=dom('pageContent'),pa=dom('pageActions');
  if(!CURRENT_DIR){el.innerHTML='<div class="empty-state"><div class="icon">📁</div><h3>Select a Directory</h3></div>';pa.innerHTML='';return;}
  pa.innerHTML='<select id="repurposeSelect" style="padding:6px 10px;border:1px solid var(--border);border-radius:6px;font-size:.85rem" onchange="doRepurpose()"><option value="">Select a format...</option><option value="faq">🗣️ FAQ Section</option><option value="email_teaser">📧 Email Teaser</option><option value="social_pack">📱 Social Media Pack (Short/Medium/Long)</option></select>';
  el.innerHTML='<div style="text-align:center;padding:40px"><span class="spinner-teal"></span></div>';
  try{
    var posts=await g("/api/v1/directories/"+CURRENT_DIR+"/blog-posts");
    if(!posts.length){el.innerHTML='<div class="empty-state"><div class="icon">🔄</div><h3>No Blog Posts</h3><p>Write some blog posts first, then repurpose them here.</p></div>';pa.innerHTML='';return;}
    var h='<div class="card-grid">';
    for(var i=0;i<posts.length;i++){
      var p=posts[i];
      h+='<div class="card" style="cursor:pointer" onclick="selectForRepurpose(\''+p.id+'\')"><h4 style="font-size:.9rem;margin-bottom:4px">'+esc(p.title||p.slug)+'</h4><div style="font-size:.75rem;color:var(--text-secondary)">'+(p.published?'<span class="badge active">Published</span>':'<span class="badge inactive">Draft</span>')+' · '+(p.updated_at||p.created_at||'').substring(0,10)+'</div></div>';
    }
    el.innerHTML=h+'</div>';
    dom('_repurposePosts',posts);
  }catch(e){el.innerHTML='<div class="empty-state"><div class="icon">⚠️</div><h3>Error</h3><p>'+esc(e.message)+'</p></div>';}
}

async function selectForRepurpose(postId){
  var format=domById('repurposeSelect').value;
  if(!format){t('Select a format first','error');return;}
  dom('pageContent').innerHTML='<div style="text-align:center;padding:40px"><span class="spinner-teal"></span></div>';
  try{
    var result=await po("/api/v1/directories/"+CURRENT_DIR+"/repurpose",{post_id:postId,format:format});
    dom('pageContent').innerHTML='<div class="card"><h2 style="margin-bottom:16px">🔄 Repurpose Result — '+esc(format)+'</h2><pre style="background:#f1f5f9;padding:20px;border-radius:8px;font-size:.85rem;overflow:auto;max-height:60vh;white-space:pre-wrap">'+esc(JSON.stringify(result,null,2))+'</pre><button class="btn btn-sm btn-primary mt-2" onclick="copyRepurposeResult()">📋 Copy</button></div>';
    dom('_repurposeResult',JSON.stringify(result,null,2));
  }catch(e){dom('pageContent').innerHTML='<div class="card">⚠️ '+esc(e.message)+'</div>';}
}

function doRepurpose(){
  if(dom('_repurposePosts'))dom('_repurposePosts').innerHTML=dom('_repurposePosts').innerHTML;
}

function copyRepurposeResult(){
  var r=dom('_repurposeResult');if(r)navigator.clipboard.writeText(r).then(function(){t('Copied!');}).catch(function(){t('Copy failed','error');});
}

// ── DIRECTORY SWITCHING (hook into existing directories page) ──
// Override the directory manage button to set CURRENT_DIR and navigate
(function(){
  var origManage=window.manageDirectory;
  window.manageDirectory=function(id){
    CURRENT_DIR=id;
    if(origManage)origManage(id);
    // Navigate to services by default
    if(typeof navigate==='function')navigate('services');
  };
})();
