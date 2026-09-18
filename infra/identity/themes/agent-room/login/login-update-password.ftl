<#import "template.ftl" as layout>
<#import "password-commons.ftl" as passwordCommons>
<@layout.registrationLayout displayMessage=!messagesPerField.existsError('password','password-confirm'); section>
    <#if section = "header">
        ${msg("updatePasswordTitle")}
    <#elseif section = "form">
        <form id="kc-passwd-update-form" class="ar-form" onsubmit="login.disabled = true; return true;" action="${url.loginAction}" method="post">
            <#list ["password-new", "password-confirm"] as field>
                <div class="ar-field">
                    <label for="${field}" class="ar-label"><#if field == "password-new">${msg("passwordNew")}<#else>${msg("passwordConfirm")}</#if></label>
                    <div class="ar-password" dir="ltr">
                        <input id="${field}" class="ar-input" name="${field}" type="password" autocomplete="new-password"<#if field == "password-new"> autofocus aria-describedby="password-hint"</#if>
                               aria-invalid="<#if messagesPerField.existsError('password','password-confirm')>true</#if>"/>
                        <button class="ar-password-toggle" type="button" aria-label="${msg('showPassword')}"
                                aria-controls="${field}" data-password-toggle
                                data-icon-show="${properties.kcFormPasswordVisibilityIconShow!}" data-icon-hide="${properties.kcFormPasswordVisibilityIconHide!}"
                                data-label-show="${msg('showPassword')}" data-label-hide="${msg('hidePassword')}">
                            <i class="${properties.kcFormPasswordVisibilityIconShow!}" aria-hidden="true"></i>
                        </button>
                    </div>
                    <#if field == "password-new">
                        <span id="password-hint" class="ar-hint">${msg("arPasswordHint")}</span>
                    </#if>
                </div>
            </#list>
            <#if messagesPerField.existsError('password','password-confirm')>
                <span id="input-error-password" class="ar-field-error" aria-live="polite">${kcSanitize(messagesPerField.getFirstError('password','password-confirm'))?no_esc}</span>
            </#if>

            <@passwordCommons.logoutOtherSessions/>

            <#if isAppInitiatedAction??>
                <div class="ar-actions">
                    <button class="ar-button ar-button--primary" name="login" type="submit">${msg("doSubmit")}</button>
                    <button class="ar-button ar-button--secondary" type="submit" name="cancel-aia" value="true">${msg("doCancel")}</button>
                </div>
            <#else>
                <button class="ar-button ar-button--primary ar-button--block" name="login" type="submit">${msg("arSavePassword")}</button>
            </#if>
        </form>
        <script type="module" src="${url.resourcesPath}/js/passwordVisibility.js"></script>
    </#if>
</@layout.registrationLayout>
